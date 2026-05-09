/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The Rust parser is being staged bottom-up. Keep this module warning-free until
// the C++ bridge starts calling into it.
#![allow(dead_code)]

use crate::css_tokenizer::{CssNumberType, CssToken, NumericValue, Token, TokenType};
use crate::generated_descriptors::{
    DescriptorSyntax, at_rule_id_from_u8, at_rule_supports_descriptor as generated_at_rule_supports_descriptor,
    descriptor_allows_arbitrary_substitution_functions as generated_descriptor_allows_arbitrary_substitution_functions,
    descriptor_id_from_u8, for_each_descriptor_syntax as generated_for_each_descriptor_syntax,
};
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

#[path = "css_parser/parser_arbitrary_substitutions.rs"]
mod parser_arbitrary_substitutions;
#[path = "css_parser/parser_component_values.rs"]
mod parser_component_values;
#[path = "css_parser/parser_descriptors.rs"]
mod parser_descriptors;
#[path = "css_parser/parser_emitters.rs"]
mod parser_emitters;
#[path = "css_parser/parser_entrypoints.rs"]
mod parser_entrypoints;
#[path = "css_parser/parser_math.rs"]
mod parser_math;
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

pub(crate) use parser_arbitrary_substitutions::{
    collect_substitution_function_presence, parse_arbitrary_substitution_function_declaration_value_arguments,
    parse_arbitrary_substitution_function_if_arguments,
};
pub(crate) use parser_descriptors::*;
use parser_emitters::*;
pub(crate) use parser_entrypoints::*;
pub(crate) use parser_math::*;
use parser_selectors::*;
pub(crate) use parser_shared::serialize_component_values_for_reparsing;
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
use style_value_emitter::emit_rust_owned_style_value_with_calculation_callback;
use style_value_longhands::*;
pub(crate) use style_value_parser::component_values_match_syntax;
use style_value_parser::{
    component_values_parse_as_generated_property_value_type, component_values_parse_as_property_value_type,
    generated_property_value_type_order, parse_rust_owned_style_value_for_property,
    parse_rust_owned_style_value_for_property_with_mode, parse_rust_owned_style_value_for_property_with_options,
};
#[cfg(test)]
use style_value_parser::{
    component_values_parse_as_property_value_type_with_options, parse_rust_owned_generated_longhand_value,
    property_uses_rust_owned_whole_grammar,
};
use style_value_shorthands::parse_rust_owned_grid_template_areas_value;
#[cfg(test)]
pub(crate) use style_value_shorthands::{
    parse_coordinating_value_list_shorthand, parse_font_shorthand, parse_grid_placement_shorthand,
    parse_grid_template_shorthand, parse_layer_shorthand, parse_positional_value_list_shorthand,
    parse_rust_owned_coordinating_value_list_shorthand, parse_rust_owned_positional_value_list_shorthand,
};
use style_values::*;

pub(crate) fn emit_component_values<F>(component_values: &[ComponentValue], filtered_input: &str, callback: &mut F)
where
    F: FnMut(CssComponentValue),
{
    for component_value in component_values {
        emit_component_value(component_value, filtered_input, callback);
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDescriptorValueType {
    CounterStyleSystem,
    CounterStyleAdditiveSymbols,
    CounterStyleName,
    CounterStyleNegative,
    CounterStylePad,
    CounterStyleRange,
    CropOrCross,
    FamilyName,
    FontSrcList,
    FontWeightAbsolutePair,
    Length,
    OptionalDeclarationValue,
    PageSize,
    PositivePercentage,
    String,
    Symbol,
    Symbols,
    UnicodeRangeTokens,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDescriptorSyntaxKind {
    Keyword,
    Property,
    ValueType,
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

fn serialize_family_name_source(name: &str, is_string: bool) -> String {
    if !is_string {
        return name.to_string();
    }

    let mut serialized = String::with_capacity(name.len() + 2);
    serialized.push('"');
    for character in name.chars() {
        match character {
            '\\' | '"' => {
                serialized.push('\\');
                serialized.push(character);
            }
            '\n' => serialized.push_str("\\a "),
            '\r' => serialized.push_str("\\d "),
            '\u{c}' => serialized.push_str("\\c "),
            _ => serialized.push(character),
        }
    }
    serialized.push('"');
    serialized
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

pub(crate) fn descriptor_supports(at_rule_id: u8, descriptor_id: u8) -> bool {
    let Some(at_rule_id) = at_rule_id_from_u8(at_rule_id) else {
        return false;
    };
    let Some(descriptor_id) = descriptor_id_from_u8(descriptor_id) else {
        return false;
    };
    generated_at_rule_supports_descriptor(at_rule_id, descriptor_id)
}

pub(crate) fn descriptor_allows_arbitrary_substitution_functions(at_rule_id: u8, descriptor_id: u8) -> bool {
    let Some(at_rule_id) = at_rule_id_from_u8(at_rule_id) else {
        return false;
    };
    let Some(descriptor_id) = descriptor_id_from_u8(descriptor_id) else {
        return false;
    };
    generated_descriptor_allows_arbitrary_substitution_functions(at_rule_id, descriptor_id)
}

pub(crate) fn for_each_descriptor_syntax<F>(at_rule_id: u8, descriptor_id: u8, mut callback: F) -> bool
where
    F: FnMut(CssDescriptorSyntaxKind, u16, CssDescriptorValueType, &str),
{
    let Some(at_rule_id) = at_rule_id_from_u8(at_rule_id) else {
        return false;
    };
    let Some(descriptor_id) = descriptor_id_from_u8(descriptor_id) else {
        return false;
    };

    generated_for_each_descriptor_syntax(at_rule_id, descriptor_id, |syntax| match syntax {
        DescriptorSyntax::Keyword(keyword) => callback(
            CssDescriptorSyntaxKind::Keyword,
            0,
            CssDescriptorValueType::CounterStyleSystem,
            keyword,
        ),
        DescriptorSyntax::Property(property_id) => callback(
            CssDescriptorSyntaxKind::Property,
            property_id as u16,
            CssDescriptorValueType::CounterStyleSystem,
            "",
        ),
        DescriptorSyntax::ValueType(value_type) => callback(CssDescriptorSyntaxKind::ValueType, 0, value_type, ""),
    })
}

pub(crate) struct DescriptorResultCallbacks<K, S, C, F, U, M, O, T> {
    pub(crate) kind_callback: K,
    pub(crate) source_callback: S,
    pub(crate) calculation_callback: C,
    pub(crate) font_source_callback: F,
    pub(crate) url_callback: U,
    pub(crate) modifier_callback: M,
    pub(crate) format_callback: O,
    pub(crate) tech_callback: T,
}

pub(crate) fn parse_descriptor_result<K, S, C, F, U, M, O, T>(
    value_type: CssDescriptorValueType,
    filtered_input: &[u8],
    mut callbacks: DescriptorResultCallbacks<K, S, C, F, U, M, O, T>,
) -> bool
where
    K: FnMut(CssDescriptorResultKind),
    S: for<'a> FnMut(CssNonnegativeIntegerSymbolPairOrder, &'a str, bool, CssPrimitiveValueKind, bool, f64, u8, u8),
    C: for<'a> FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &'a [u8]),
    F: for<'a> FnMut(CssFontSourceKind, Option<&'a str>, bool),
    U: FnMut(CssUrlFunction),
    M: FnMut(CssUrlModifier),
    O: for<'a> FnMut(&'a str),
    T: FnMut(CssFontTech),
{
    let default_order = CssNonnegativeIntegerSymbolPairOrder::IntegerFirst;

    match value_type {
        CssDescriptorValueType::CounterStyleAdditiveSymbols => {
            let Some(tuples) = parse_rust_owned_counter_style_additive_symbols_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleAdditiveSymbols);
            for tuple in &tuples {
                (callbacks.source_callback)(
                    tuple.order,
                    tuple.symbol.source_or_unit(),
                    false,
                    tuple.symbol.primitive_kind(),
                    tuple.integer.is_some(),
                    tuple.integer.map(f64::from).unwrap_or(0.0),
                    0,
                    0,
                );
                if let Some(calculation) = &tuple.integer_calculation {
                    emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
                }
            }
        }
        CssDescriptorValueType::CounterStyleNegative => {
            let Some(symbols) = parse_rust_owned_counter_style_negative_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleNegative);
            for symbol in &symbols {
                (callbacks.source_callback)(
                    default_order,
                    symbol.source_or_unit(),
                    false,
                    symbol.primitive_kind(),
                    false,
                    0.0,
                    0,
                    0,
                );
            }
        }
        CssDescriptorValueType::CounterStyleSystem => {
            let Some(system) = parse_rust_owned_counter_style_system_descriptor(filtered_input) else {
                return false;
            };

            match system {
                RustOwnedCounterStyleSystemDescriptor::Cyclic => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemCyclic);
                }
                RustOwnedCounterStyleSystemDescriptor::Numeric => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemNumeric);
                }
                RustOwnedCounterStyleSystemDescriptor::Alphabetic => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemAlphabetic);
                }
                RustOwnedCounterStyleSystemDescriptor::Symbolic => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemSymbolic);
                }
                RustOwnedCounterStyleSystemDescriptor::Additive => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemAdditive);
                }
                RustOwnedCounterStyleSystemDescriptor::Fixed { first_symbol } => {
                    if let Some(first_symbol) = first_symbol {
                        (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemFixedWithInteger);
                        (callbacks.source_callback)(
                            default_order,
                            first_symbol.source_or_unit(),
                            false,
                            first_symbol.primitive_kind(),
                            first_symbol.has_numeric_value(),
                            first_symbol.numeric_value(),
                            0,
                            0,
                        );
                        if let Some(calculation) = &first_symbol.calculation {
                            emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
                        }
                    } else {
                        (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemFixed);
                    }
                }
                RustOwnedCounterStyleSystemDescriptor::Extends { name } => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleSystemExtends);
                    (callbacks.source_callback)(
                        default_order,
                        &name,
                        false,
                        CssPrimitiveValueKind::Invalid,
                        false,
                        0.0,
                        0,
                        0,
                    );
                }
            }
        }
        CssDescriptorValueType::CounterStylePad => {
            let Some(pad) = parse_rust_owned_counter_style_pad_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::CounterStylePad);
            (callbacks.source_callback)(
                pad.order,
                pad.symbol.source_or_unit(),
                false,
                pad.symbol.primitive_kind(),
                pad.integer.is_some(),
                pad.integer.map(f64::from).unwrap_or(0.0),
                0,
                0,
            );
            if let Some(calculation) = &pad.integer_calculation {
                emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
            }
        }
        CssDescriptorValueType::CounterStyleRange => {
            let Some(range) = parse_rust_owned_counter_style_range_descriptor(filtered_input) else {
                return false;
            };

            match range {
                RustOwnedCounterStyleRangeDescriptor::Auto => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleRangeAuto);
                }
                RustOwnedCounterStyleRangeDescriptor::List(ranges) => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CounterStyleRangeList);
                    for bound in &ranges {
                        (callbacks.source_callback)(
                            default_order,
                            bound.source_or_unit(),
                            false,
                            bound.primitive_kind(),
                            bound.has_numeric_value(),
                            bound.numeric_value(),
                            0,
                            0,
                        );
                        if let Some(calculation) = &bound.calculation {
                            emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
                        }
                    }
                }
            }
        }
        CssDescriptorValueType::Symbols => {
            let Some(symbols) = parse_rust_owned_counter_style_symbols_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::Symbols);
            for symbol in &symbols {
                (callbacks.source_callback)(
                    default_order,
                    symbol.source_or_unit(),
                    false,
                    symbol.primitive_kind(),
                    false,
                    0.0,
                    0,
                    0,
                );
            }
        }
        CssDescriptorValueType::Symbol => {
            let Some(symbol) = parse_rust_owned_counter_style_symbol_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::Symbol);
            (callbacks.source_callback)(
                default_order,
                symbol.source_or_unit(),
                false,
                symbol.primitive_kind(),
                false,
                0.0,
                0,
                0,
            );
        }
        CssDescriptorValueType::CropOrCross => {
            let mut kind = None;
            if !parse_crop_or_cross(filtered_input, |parsed_kind| kind = Some(parsed_kind)) {
                return false;
            }

            match kind {
                Some(CssCropOrCrossKind::Crop) => (callbacks.kind_callback)(CssDescriptorResultKind::Crop),
                Some(CssCropOrCrossKind::Cross) => (callbacks.kind_callback)(CssDescriptorResultKind::Cross),
                Some(CssCropOrCrossKind::CropAndCross) => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::CropAndCross);
                }
                None => return false,
            }
        }
        CssDescriptorValueType::FamilyName => {
            let mut family_name = None;
            if !parse_a_family_name(filtered_input, |name, is_string| {
                family_name = Some((name.to_string(), is_string));
            }) {
                return false;
            }
            let Some((family_name, is_string)) = family_name else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::FamilyName);
            (callbacks.source_callback)(
                default_order,
                &family_name,
                is_string,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                0,
                0,
            );
        }
        CssDescriptorValueType::FontSrcList => {
            let Some(sources) = parse_rust_owned_font_src_list_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::FontSrcList);
            for source in &sources {
                (callbacks.source_callback)(
                    default_order,
                    &source.source,
                    false,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    0,
                    0,
                );
                match &source.font_source {
                    FontSource::Local(family_name) => {
                        (callbacks.font_source_callback)(
                            CssFontSourceKind::Local,
                            Some(&family_name.name),
                            family_name.is_string,
                        );
                    }
                    FontSource::Url {
                        url_function,
                        format,
                        tech,
                    } => {
                        (callbacks.font_source_callback)(CssFontSourceKind::Url, None, false);
                        (callbacks.url_callback)(CssUrlFunction {
                            function_type: url_function.function_type,
                            url_ptr: url_function.url.as_ptr(),
                            url_len: url_function.url.len(),
                        });
                        for modifier in &url_function.request_url_modifiers {
                            (callbacks.modifier_callback)(modifier.as_ffi());
                        }
                        if let Some(format) = format {
                            (callbacks.format_callback)(format);
                        }
                        for font_tech in tech {
                            (callbacks.tech_callback)(*font_tech);
                        }
                    }
                }
            }
        }
        CssDescriptorValueType::FontWeightAbsolutePair => {
            let Some(weights) = parse_rust_owned_font_weight_absolute_pair_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::FontWeightAbsolutePair);
            for weight in &weights {
                (callbacks.source_callback)(
                    default_order,
                    weight.source_or_unit(),
                    false,
                    weight.primitive_kind(),
                    weight.has_numeric_value(),
                    weight.numeric_value(),
                    0,
                    0,
                );
                if let Some(calculation) = &weight.calculation {
                    emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
                }
            }
        }
        CssDescriptorValueType::Length => {
            let Some(value) = parse_rust_owned_length_descriptor_value(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::Length);
            (callbacks.source_callback)(
                default_order,
                value.source_or_unit(),
                false,
                value.primitive_kind(),
                value.has_numeric_value(),
                value.numeric_value(),
                0,
                0,
            );
            if let Some(calculation) = &value.calculation {
                emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
            }
        }
        CssDescriptorValueType::PageSize => {
            let Some(page_size) = parse_rust_owned_page_size_descriptor(filtered_input) else {
                return false;
            };

            match page_size {
                RustOwnedPageSizeDescriptor::Auto => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::PageSizeAuto);
                }
                RustOwnedPageSizeDescriptor::Lengths(lengths) => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::PageSizeLengths);
                    for length in &lengths {
                        (callbacks.source_callback)(
                            default_order,
                            length.source_or_unit(),
                            false,
                            length.primitive_kind(),
                            length.has_numeric_value(),
                            length.numeric_value(),
                            0,
                            0,
                        );
                        if let Some(calculation) = &length.calculation {
                            emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
                        }
                    }
                }
                RustOwnedPageSizeDescriptor::PageSizeAndOrientation { page_size, orientation } => {
                    (callbacks.kind_callback)(CssDescriptorResultKind::PageSizeAndOrientation);
                    if let Some(page_size) = page_size {
                        (callbacks.source_callback)(
                            default_order,
                            "",
                            false,
                            CssPrimitiveValueKind::Invalid,
                            false,
                            0.0,
                            page_size as u8 + 1,
                            0,
                        );
                    }
                    if let Some(orientation) = orientation {
                        (callbacks.source_callback)(
                            default_order,
                            "",
                            false,
                            CssPrimitiveValueKind::Invalid,
                            false,
                            0.0,
                            0,
                            orientation as u8 + 1,
                        );
                    }
                }
            }
        }
        CssDescriptorValueType::PositivePercentage => {
            let Some(value) = parse_rust_owned_positive_percentage_descriptor_value(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::PositivePercentage);
            (callbacks.source_callback)(
                default_order,
                value.source_or_unit(),
                false,
                value.primitive_kind(),
                value.has_numeric_value(),
                value.numeric_value(),
                0,
                0,
            );
            if let Some(calculation) = &value.calculation {
                emit_rust_owned_calculation_tree(calculation, &mut callbacks.calculation_callback);
            }
        }
        CssDescriptorValueType::String => {
            let Some(source) = parse_rust_owned_string_descriptor(filtered_input) else {
                return false;
            };

            (callbacks.kind_callback)(CssDescriptorResultKind::String);
            (callbacks.source_callback)(
                default_order,
                &source,
                true,
                CssPrimitiveValueKind::String,
                false,
                0.0,
                0,
                0,
            );
        }
        _ => return false,
    }

    true
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
