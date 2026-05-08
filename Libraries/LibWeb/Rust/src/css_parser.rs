/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The Rust parser is being staged bottom-up. Keep this module warning-free until
// the C++ bridge starts calling into it.
#![allow(dead_code)]

use crate::css_tokenizer::{CssNumberType, CssToken, NumericValue, Token, TokenType};
use crate::generated_media_features::{
    MediaFeatureId, MediaFeatureValueType, media_feature_accepts_identifier, media_feature_accepts_type,
    media_feature_id_from_string, media_feature_type_is_range,
};
use crate::generated_properties::{
    PropertyId, PropertyNumericRange, PropertyValueType, longhands_for_shorthand,
    property_accepted_range_by_value_type, property_accepts_keyword, property_accepts_value_type,
    property_custom_ident_blacklist, property_id_from_u16, property_is_positional_value_list_shorthand,
    property_resolves_percentages_relative_to, property_value_type_from_css_value_type_name, property_value_type_name,
    resolve_legacy_value_alias,
};
use crate::generated_pseudo_classes::{
    PseudoClassId, PseudoClassParameterType, pseudo_class_id_from_string, pseudo_class_metadata,
};
use crate::generated_pseudo_elements::{
    PseudoElementId, PseudoElementParameterType, aliased_pseudo_element_id_from_string, is_has_allowed_pseudo_element,
    pseudo_element_id_from_string, pseudo_element_metadata,
};
use crate::generated_transform_functions::{
    TransformFunction, TransformFunctionParameterType, transform_function_from_name, transform_function_parameters,
    transform_function_parameters_from_name,
};
use crate::generated_units::{DimensionType, dimension_for_unit};
use crate::generated_value_types::{
    GeneratedValueTypeStyleValueKind, ValueTypeId, component_values_parse_as_generated_value_type,
    generated_value_type_style_value, value_type_id_from_u8,
};

#[path = "css_parser/parser_component_values.rs"]
mod parser_component_values;
#[path = "css_parser/parser_emitters.rs"]
mod parser_emitters;
#[path = "css_parser/parser_entrypoints.rs"]
mod parser_entrypoints;
#[path = "css_parser/parser_selectors.rs"]
mod parser_selectors;
#[path = "css_parser/parser_shared.rs"]
mod parser_shared;
#[path = "css_parser/parser_syntax_media.rs"]
mod parser_syntax_media;
#[path = "css_parser/parser_token_stream.rs"]
mod parser_token_stream;
#[path = "css_parser/parser_types.rs"]
mod parser_types;
#[path = "css_parser/parser_urls_fonts.rs"]
mod parser_urls_fonts;
#[path = "css_parser/style_value_emitter.rs"]
mod style_value_emitter;
#[path = "css_parser/style_value_longhands.rs"]
mod style_value_longhands;
#[path = "css_parser/style_value_parser.rs"]
mod style_value_parser;
#[path = "css_parser/style_value_shorthands.rs"]
mod style_value_shorthands;
#[path = "css_parser/style_values.rs"]
mod style_values;

use parser_emitters::*;
pub(crate) use parser_entrypoints::*;
use parser_selectors::*;
pub(crate) use parser_shared::strip_whitespace;
use parser_shared::*;
use parser_syntax_media::*;
pub(crate) use parser_syntax_media::{
    component_values_custom_ident_value, component_values_number_value, component_values_parse_as_custom_ident,
    component_values_parse_as_ident, component_values_parse_as_number, component_values_parse_as_string,
    component_values_string_value,
};
use parser_token_stream::*;
pub use parser_types::*;
use parser_urls_fonts::*;
use style_value_emitter::emit_rust_owned_style_value;
use style_value_longhands::*;
#[cfg(test)]
use style_value_parser::parse_rust_owned_generated_longhand_value;
use style_value_parser::{
    component_values_parse_as_generated_property_value_type, component_values_parse_as_property_value_type,
    generated_property_value_type_order, parse_rust_owned_style_value_for_property,
    parse_rust_owned_style_value_for_property_with_mode,
};
use style_value_shorthands::parse_rust_owned_grid_template_areas_value;
pub(crate) use style_value_shorthands::{
    parse_coordinating_value_list_shorthand, parse_font_shorthand, parse_grid_placement_shorthand,
    parse_grid_template_shorthand, parse_layer_shorthand, parse_positional_value_list_shorthand,
};
#[cfg(test)]
pub(crate) use style_value_shorthands::{
    parse_rust_owned_coordinating_value_list_shorthand, parse_rust_owned_positional_value_list_shorthand,
};
use style_values::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSyntaxNodeKind {
    Invalid,
    Universal,
    Type,
    Ident,
    MultiplierStart,
    MultiplierEnd,
    CommaSeparatedMultiplierStart,
    CommaSeparatedMultiplierEnd,
    AlternativesStart,
    AlternativesEnd,
}

#[repr(C)]
pub struct CssSyntaxNode {
    pub kind: CssSyntaxNodeKind,
    pub value_ptr: *const u8,
    pub value_len: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FamilyName {
    pub(crate) name: String,
    pub(crate) is_string: bool,
}

pub(crate) fn parse_a_list_of_component_values<F>(filtered_input: &[u8], mut callback: F)
where
    F: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    for component_value in parser.parse_a_list_of_component_values() {
        emit_component_value(&component_value, filtered_input_string, &mut callback);
    }
}

pub(crate) fn parse_a_comma_separated_list_of_component_values<G, C>(
    filtered_input: &[u8],
    mut group_callback: G,
    mut component_value_callback: C,
) where
    G: FnMut(),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    for group in parser.parse_a_comma_separated_list_of_component_values() {
        for component_value in group {
            emit_component_value(&component_value, filtered_input_string, &mut component_value_callback);
        }
        group_callback();
    }
}

pub(crate) fn parse_empty_prelude(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    strip_whitespace(&component_values).is_empty()
}

pub(crate) fn parse_a_value_type(filtered_input: &[u8], value_type_id: u8) -> CssValueTypeSyntaxKind {
    let Some(value_type_id) = value_type_id_from_u8(value_type_id) else {
        return CssValueTypeSyntaxKind::Invalid;
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    component_values_parse_as_value_type(value_type_id, &component_values)
}

pub(crate) fn parse_property_keyword_value<C>(property_ids: &[u16], keyword: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let Ok(keyword) = std::str::from_utf8(keyword) else {
        return false;
    };

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_keyword(property_id, keyword) {
            continue;
        }

        let resolved_keyword = resolve_legacy_value_alias(property_id, keyword).unwrap_or(keyword);
        callback(property_id as u16, resolved_keyword);
        return true;
    }

    false
}

pub(crate) fn property_accepting_type<C>(property_ids: &[u16], value_type: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16),
{
    let Ok(value_type) = std::str::from_utf8(value_type) else {
        return false;
    };
    let Some(value_type) = property_value_type_from_css_value_type_name(value_type) else {
        return false;
    };

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_value_type(property_id, value_type) {
            continue;
        }

        callback(property_id as u16);
        return true;
    }

    false
}

pub(crate) fn parse_property_custom_ident_value<C>(property_ids: &[u16], filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_value_type(property_id, crate::generated_properties::PropertyValueType::CustomIdent) {
            continue;
        }

        let mut parser = ComponentValueParser::new(component_values.clone());
        let Some(name) = parser.parse_a_custom_ident(property_custom_ident_blacklist(property_id)) else {
            continue;
        };

        callback(property_id as u16, &name);
        return true;
    }

    false
}

pub(crate) fn parse_generated_property_value<C>(property_ids: &[u16], filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(CssGeneratedPropertyValueKind, u16, &[u8], &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    {
        for property_id in property_ids {
            let Some(property_id) = property_id_from_u16(*property_id) else {
                continue;
            };
            if property_accepts_keyword(property_id, value) {
                let resolved_keyword = resolve_legacy_value_alias(property_id, value).unwrap_or(value);
                callback(
                    CssGeneratedPropertyValueKind::Keyword,
                    property_id as u16,
                    resolved_keyword.as_bytes(),
                    "",
                );
                return true;
            }
        }
    }

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_value_type(property_id, PropertyValueType::CustomIdent) {
            continue;
        }

        let mut parser = ComponentValueParser::new(component_values.to_vec());
        if let Some(name) = parser.parse_a_custom_ident(property_custom_ident_blacklist(property_id)) {
            callback(
                CssGeneratedPropertyValueKind::CustomIdent,
                property_id as u16,
                name.as_bytes(),
                property_value_type_name(PropertyValueType::CustomIdent),
            );
            return true;
        }
    }

    for value_type in generated_property_value_type_order() {
        for property_id in property_ids {
            let Some(property_id) = property_id_from_u16(*property_id) else {
                continue;
            };
            if !property_accepts_value_type(property_id, *value_type) {
                continue;
            }
            let value_type_matches = if *value_type == PropertyValueType::Url {
                match property_id {
                    PropertyId::ClipPath => parse_a_url_function(filtered_input, |_| {}, |_| {}),
                    PropertyId::MaskImage => component_values_parse_as_fragment_url(filtered_input),
                    _ => false,
                }
            } else {
                component_values_parse_as_property_value_type(*value_type, filtered_input)
            };
            if !value_type_matches {
                continue;
            }

            callback(
                CssGeneratedPropertyValueKind::ValueType,
                property_id as u16,
                &[],
                property_value_type_name(*value_type),
            );
            return true;
        }
    }

    false
}

#[cfg(test)]
#[path = "css_parser/tests.rs"]
mod tests;
