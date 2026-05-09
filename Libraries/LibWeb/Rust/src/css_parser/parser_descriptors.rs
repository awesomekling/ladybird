/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedCounterStyleRangeDescriptor {
    Auto,
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterStyleAdditiveTuple {
    pub(crate) order: CssNonnegativeIntegerSymbolPairOrder,
    pub(crate) source: String,
    pub(crate) integer: Option<i32>,
    pub(crate) symbol: RustOwnedDescriptorPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterStylePadDescriptor {
    pub(crate) order: CssNonnegativeIntegerSymbolPairOrder,
    pub(crate) source: String,
    pub(crate) integer: Option<i32>,
    pub(crate) symbol: RustOwnedDescriptorPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedCounterStyleSystemDescriptor {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Additive,
    Fixed {
        first_symbol: Option<RustOwnedDescriptorPrimitiveValue>,
    },
    Extends {
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPageSizeDescriptor {
    Auto,
    Lengths(Vec<RustOwnedDescriptorPrimitiveValue>),
    PageSizeAndOrientation {
        page_size: Option<CssPageSizeKeyword>,
        orientation: Option<CssPageSizeOrientation>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedDescriptorPrimitiveValue {
    pub(crate) primitive_kind: CssPrimitiveValueKind,
    pub(crate) numeric_value: Option<f64>,
    pub(crate) source_or_unit: String,
}

impl RustOwnedDescriptorPrimitiveValue {
    pub(crate) fn primitive_kind(&self) -> CssPrimitiveValueKind {
        self.primitive_kind
    }

    pub(crate) fn has_numeric_value(&self) -> bool {
        self.numeric_value.is_some()
    }

    pub(crate) fn numeric_value(&self) -> f64 {
        self.numeric_value.unwrap_or(0.0)
    }

    pub(crate) fn source_or_unit(&self) -> &str {
        &self.source_or_unit
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedNonnegativeIntegerSymbolPair {
    pub(crate) order: CssNonnegativeIntegerSymbolPairOrder,
    pub(crate) source: String,
    pub(crate) integer: Option<i32>,
    pub(crate) symbol: RustOwnedDescriptorPrimitiveValue,
}

pub(crate) fn parse_rust_owned_counter_style_negative_descriptor(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedDescriptorPrimitiveValue>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let mut symbols = Vec::new();

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative
    // <symbol> <symbol>?
    parser.discard_whitespace();
    symbols.push(parser.consume_symbol_value()?);

    parser.discard_whitespace();
    if let Some(symbol) = parser.consume_symbol_value() {
        symbols.push(symbol);
    }

    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(symbols)
}

pub(crate) fn parse_rust_owned_counter_style_symbols_descriptor(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedDescriptorPrimitiveValue>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let mut symbols = Vec::new();

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
    // <symbol>+
    loop {
        parser.discard_whitespace();
        let Some(symbol) = parser.consume_symbol_value() else {
            break;
        };
        symbols.push(symbol);
    }

    parser.discard_whitespace();
    (!symbols.is_empty() && !parser.has_next_component_value()).then_some(symbols)
}

pub(crate) fn parse_rust_owned_counter_style_range_descriptor(
    filtered_input: &[u8],
) -> Option<RustOwnedCounterStyleRangeDescriptor> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-range
    // [ [ <integer> | infinite ]{2} ]# | auto
    parser.discard_whitespace();
    if parser.consume_ident_matching("auto") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedCounterStyleRangeDescriptor::Auto);
    }

    let mut ranges = Vec::new();
    loop {
        parser.discard_whitespace();
        let start = parser.index;
        if !parser.consume_counter_style_range_bound_syntax() {
            break;
        }

        parser.discard_whitespace();
        if !parser.consume_counter_style_range_bound_syntax() {
            return None;
        }

        ranges.push(serialize_component_values_for_reparsing(
            &parser.component_values[start..parser.index],
            &filtered_input,
        )?);

        parser.discard_whitespace();
        if !parser.consume_comma() {
            break;
        }
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            return None;
        }
    }

    parser.discard_whitespace();
    (!ranges.is_empty() && !parser.has_next_component_value())
        .then_some(RustOwnedCounterStyleRangeDescriptor::List(ranges))
}

pub(crate) fn parse_rust_owned_counter_style_additive_symbols_descriptor(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedCounterStyleAdditiveTuple>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);
    let mut tuples = Vec::new();

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-symbols
    // <additive-symbols> = <additive-tuple>#
    loop {
        parser.discard_whitespace();
        let pair = parser.parse_a_nonnegative_integer_symbol_pair_value(&filtered_input)?;
        tuples.push(RustOwnedCounterStyleAdditiveTuple {
            order: pair.order,
            source: pair.source,
            integer: pair.integer,
            symbol: pair.symbol,
        });

        parser.discard_whitespace();
        if !parser.consume_comma() {
            break;
        }
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            return None;
        }
    }

    parser.discard_whitespace();
    (!tuples.is_empty() && !parser.has_next_component_value()).then_some(tuples)
}

pub(crate) fn parse_rust_owned_counter_style_system_descriptor(
    filtered_input: &[u8],
) -> Option<RustOwnedCounterStyleSystemDescriptor> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-system
    // cyclic | numeric | alphabetic | symbolic | additive | [fixed <integer>?] | [ extends <counter-style-name> ]
    parser.discard_whitespace();

    let descriptor = if parser.consume_ident_matching("cyclic") {
        RustOwnedCounterStyleSystemDescriptor::Cyclic
    } else if parser.consume_ident_matching("numeric") {
        RustOwnedCounterStyleSystemDescriptor::Numeric
    } else if parser.consume_ident_matching("alphabetic") {
        RustOwnedCounterStyleSystemDescriptor::Alphabetic
    } else if parser.consume_ident_matching("symbolic") {
        RustOwnedCounterStyleSystemDescriptor::Symbolic
    } else if parser.consume_ident_matching("additive") {
        RustOwnedCounterStyleSystemDescriptor::Additive
    } else if parser.consume_ident_matching("fixed") {
        parser.discard_whitespace();
        let first_symbol = if let Some(first_symbol) = parser.consume_integer_value() {
            Some(RustOwnedDescriptorPrimitiveValue {
                primitive_kind: first_symbol
                    .map(|_| CssPrimitiveValueKind::Integer)
                    .unwrap_or(CssPrimitiveValueKind::Invalid),
                numeric_value: first_symbol.map(f64::from),
                source_or_unit: serialize_component_values_for_reparsing(
                    &parser.component_values[(parser.index - 1)..parser.index],
                    &filtered_input,
                )?,
            })
        } else {
            None
        };
        RustOwnedCounterStyleSystemDescriptor::Fixed { first_symbol }
    } else if parser.consume_ident_matching("extends") {
        parser.discard_whitespace();
        let name = parser.parse_a_counter_style_name()?;
        RustOwnedCounterStyleSystemDescriptor::Extends { name }
    } else {
        return None;
    };

    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(descriptor)
}

pub(crate) fn parse_rust_owned_counter_style_pad_descriptor(
    filtered_input: &[u8],
) -> Option<RustOwnedCounterStylePadDescriptor> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-pad
    // <integer [0,∞]> && <symbol>
    parser.discard_whitespace();
    let pair = parser.parse_a_nonnegative_integer_symbol_pair_value(&filtered_input)?;
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    Some(RustOwnedCounterStylePadDescriptor {
        order: pair.order,
        source: pair.source,
        integer: pair.integer,
        symbol: pair.symbol,
    })
}

pub(crate) fn parse_rust_owned_counter_style_symbol_descriptor(
    filtered_input: &[u8],
) -> Option<RustOwnedDescriptorPrimitiveValue> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
    // <symbol> = <string> | <image> | <custom-ident>
    parser.discard_whitespace();
    let symbol = parser.consume_symbol_value()?;
    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(symbol)
}

pub(crate) fn parse_rust_owned_font_src_list_descriptor(filtered_input: &[u8]) -> Option<Vec<String>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    // https://drafts.csswg.org/css-fonts-4/#font-face-src-parsing
    // "If parsing a component value results in a parsing error or its format
    // or tech are unsupported, do not add it to the list of supported sources."
    //
    // "If there are no supported entries at the end of this process, the value
    // for the src descriptor is a parse error."
    //
    // <font-src> = <url> [ format(<font-format>)]? [ tech( <font-tech>#)]? | local(<family-name>)
    let sources = split_component_values_on_comma(&component_values)
        .into_iter()
        .try_fold(Vec::new(), |mut sources, source| {
            let source = strip_whitespace(source);
            let mut parser = ComponentValueParser::new(source.to_vec());
            if parser.parse_a_font_source().is_some() {
                sources.push(serialize_component_values_for_reparsing(source, &filtered_input)?);
            }
            Some(sources)
        })?;

    (!sources.is_empty()).then_some(sources)
}

pub(crate) fn parse_rust_owned_font_weight_absolute_pair_descriptor(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedDescriptorPrimitiveValue>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);
    let mut weights = Vec::new();

    // https://drafts.csswg.org/css-fonts-4/#font-prop-desc
    // <font-weight-absolute>{1,2}
    for _ in 0..2 {
        parser.discard_whitespace();
        let Some(weight) = parser.consume_font_weight_absolute_value(&filtered_input) else {
            break;
        };
        weights.push(weight);
    }

    parser.discard_whitespace();
    (!weights.is_empty() && !parser.has_next_component_value()).then_some(weights)
}

pub(crate) fn parse_rust_owned_length_descriptor(filtered_input: &[u8]) -> Option<String> {
    parse_rust_owned_length_descriptor_value(filtered_input).map(|value| value.source_or_unit)
}

pub(crate) fn parse_rust_owned_length_descriptor_value(
    filtered_input: &[u8],
) -> Option<RustOwnedDescriptorPrimitiveValue> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#lengths
    // <length>
    let [component_value] = component_values else {
        return None;
    };

    let value = component_value_parse_as_nested_length(component_value, &filtered_input)?;
    let (primitive_kind, numeric_value, source_or_unit) = match value {
        RustOwnedNestedPrimitiveValue::Length { value, unit } => (CssPrimitiveValueKind::Length, Some(value), unit),
        RustOwnedNestedPrimitiveValue::MathFunction(value) => (CssPrimitiveValueKind::Invalid, None, value.source),
        RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) => (
            CssPrimitiveValueKind::Invalid,
            None,
            match value.function {
                RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count()".to_string(),
                RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index()".to_string(),
            },
        ),
        RustOwnedNestedPrimitiveValue::Source(source) => (CssPrimitiveValueKind::Invalid, None, source),
        _ => return None,
    };
    Some(RustOwnedDescriptorPrimitiveValue {
        primitive_kind,
        numeric_value,
        source_or_unit,
    })
}

pub(crate) fn parse_rust_owned_positive_percentage_descriptor(filtered_input: &[u8]) -> Option<String> {
    parse_rust_owned_positive_percentage_descriptor_value(filtered_input).map(|value| value.source_or_unit)
}

pub(crate) fn parse_rust_owned_positive_percentage_descriptor_value(
    filtered_input: &[u8],
) -> Option<RustOwnedDescriptorPrimitiveValue> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#percentages
    // <percentage [0,∞]>
    let [component_value] = component_values else {
        return None;
    };

    if !component_value_parse_as_positive_percentage_descriptor(component_value) {
        return None;
    }

    let value = component_value_parse_as_nested_length_percentage(component_value, &filtered_input)?;
    let (primitive_kind, numeric_value, source_or_unit) = match value {
        RustOwnedNestedPrimitiveValue::Percentage(value) => (
            CssPrimitiveValueKind::Percentage,
            Some(value),
            serialize_component_values_for_reparsing(component_values, &filtered_input)?,
        ),
        RustOwnedNestedPrimitiveValue::MathFunction(value) => (CssPrimitiveValueKind::Invalid, None, value.source),
        RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) => (
            CssPrimitiveValueKind::Invalid,
            None,
            match value.function {
                RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count()".to_string(),
                RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index()".to_string(),
            },
        ),
        RustOwnedNestedPrimitiveValue::Source(source) => (CssPrimitiveValueKind::Invalid, None, source),
        RustOwnedNestedPrimitiveValue::Length { unit, .. } => (CssPrimitiveValueKind::Invalid, None, unit),
        _ => return None,
    };
    Some(RustOwnedDescriptorPrimitiveValue {
        primitive_kind,
        numeric_value,
        source_or_unit,
    })
}

pub(crate) fn parse_rust_owned_page_size_descriptor(filtered_input: &[u8]) -> Option<RustOwnedPageSizeDescriptor> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input = filtered_input_to_string(filtered_input);

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-page-3/#page-size-prop
    // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
    parser.discard_whitespace();
    if parser.consume_ident_matching("auto") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedPageSizeDescriptor::Auto);
    }

    let saved_index = parser.index;
    let mut lengths = Vec::new();
    for _ in 0..2 {
        parser.discard_whitespace();
        let start = parser.index;
        if !parser.consume_nonnegative_length_descriptor_syntax() {
            break;
        }
        let component_value = &parser.component_values[start];
        let value = component_value_parse_as_nested_length(component_value, &filtered_input)?;
        let (primitive_kind, numeric_value, source_or_unit) = match value {
            RustOwnedNestedPrimitiveValue::Length { value, unit } => (CssPrimitiveValueKind::Length, Some(value), unit),
            RustOwnedNestedPrimitiveValue::MathFunction(value) => (CssPrimitiveValueKind::Invalid, None, value.source),
            RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) => (
                CssPrimitiveValueKind::Invalid,
                None,
                match value.function {
                    RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count()".to_string(),
                    RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index()".to_string(),
                },
            ),
            RustOwnedNestedPrimitiveValue::Source(source) => (CssPrimitiveValueKind::Invalid, None, source),
            _ => return None,
        };
        lengths.push(RustOwnedDescriptorPrimitiveValue {
            primitive_kind,
            numeric_value,
            source_or_unit,
        });
    }
    parser.discard_whitespace();
    if !lengths.is_empty() {
        return (!parser.has_next_component_value()).then_some(RustOwnedPageSizeDescriptor::Lengths(lengths));
    }
    parser.index = saved_index;

    let mut page_size = None;
    let mut orientation = None;

    for _ in 0..2 {
        parser.discard_whitespace();
        let Some(ident) = parser.consume_an_ident() else {
            break;
        };
        if is_page_size_keyword(&ident) {
            if page_size.is_some() {
                return None;
            }
            page_size = page_size_keyword_from_string(&ident);
        } else if ident.eq_ignore_ascii_case("portrait") || ident.eq_ignore_ascii_case("landscape") {
            if orientation.is_some() {
                return None;
            }
            orientation = page_size_orientation_from_string(&ident);
        } else {
            return None;
        }
    }

    parser.discard_whitespace();
    (!parser.has_next_component_value() && (page_size.is_some() || orientation.is_some()))
        .then_some(RustOwnedPageSizeDescriptor::PageSizeAndOrientation { page_size, orientation })
}

pub(crate) fn parse_rust_owned_string_descriptor(filtered_input: &[u8]) -> Option<String> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#strings
    // <string>
    component_values_string_value(component_values).map(ToString::to_string)
}

pub(crate) fn parse_a_counter_style_name<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_counter_style_name() else {
        return false;
    };

    name_callback(&name);
    true
}

impl ComponentValueParser {
    fn consume_nonnegative_integer_value(&mut self) -> Option<Option<i32>> {
        let component_value = self.next_component_value()?;

        let integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) if number_is_integer(*number) && number.value() >= 0.0 && number.value() <= i32::MAX as f64 => {
                Some(number.value() as i32)
            }
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing and range-checking math functions still happens in C++.
            ComponentValue::Function(_) => None,
            _ => return None,
        };

        self.index += 1;
        Some(integer)
    }

    fn consume_integer_value(&mut self) -> Option<Option<i32>> {
        let component_value = self.next_component_value()?;

        let integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) if number_is_integer(*number)
                && number.value() >= i32::MIN as f64
                && number.value() <= i32::MAX as f64 =>
            {
                Some(number.value() as i32)
            }
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing math functions still happens in C++.
            ComponentValue::Function(_) => None,
            _ => return None,
        };

        self.index += 1;
        Some(integer)
    }

    fn consume_symbol_value(&mut self) -> Option<RustOwnedDescriptorPrimitiveValue> {
        let component_value = self.next_component_value()?;

        // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
        // <symbol> = <string> | <image> | <custom-ident>
        let (primitive_kind, source_or_unit) = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => (CssPrimitiveValueKind::String, value.clone()),
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &[]) => (CssPrimitiveValueKind::CustomIdent, value.clone()),
            // AD-HOC: In line with the generated <symbol> parser, we don't
            // support <image> here since that part of the grammar is at-risk
            // and unsupported by other engines.
            _ => return None,
        };

        self.index += 1;
        Some(RustOwnedDescriptorPrimitiveValue {
            primitive_kind,
            numeric_value: None,
            source_or_unit,
        })
    }

    fn parse_a_nonnegative_integer_symbol_pair_value(
        &mut self,
        filtered_input: &str,
    ) -> Option<RustOwnedNonnegativeIntegerSymbolPair> {
        // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-tuple
        // <additive-tuple> = [ <integer [0,∞]> && <symbol> ]
        let saved_index = self.index;
        if let Some(integer) = self.consume_nonnegative_integer_value() {
            self.discard_whitespace();
            if let Some(symbol) = self.consume_symbol_value() {
                return Some(RustOwnedNonnegativeIntegerSymbolPair {
                    order: CssNonnegativeIntegerSymbolPairOrder::IntegerFirst,
                    source: serialize_component_values_for_reparsing(
                        &self.component_values[saved_index..self.index],
                        filtered_input,
                    )?,
                    integer,
                    symbol,
                });
            }
        }
        self.index = saved_index;

        if let Some(symbol) = self.consume_symbol_value() {
            self.discard_whitespace();
            if let Some(integer) = self.consume_nonnegative_integer_value() {
                return Some(RustOwnedNonnegativeIntegerSymbolPair {
                    order: CssNonnegativeIntegerSymbolPairOrder::SymbolFirst,
                    source: serialize_component_values_for_reparsing(
                        &self.component_values[saved_index..self.index],
                        filtered_input,
                    )?,
                    integer,
                    symbol,
                });
            }
        }
        self.index = saved_index;
        None
    }

    fn consume_font_weight_absolute_value(
        &mut self,
        filtered_input: &str,
    ) -> Option<RustOwnedDescriptorPrimitiveValue> {
        let start = self.index;
        let component_value = self.next_component_value()?;
        let component_values = std::slice::from_ref(component_value);
        let syntax_kind =
            component_values_parse_as_generated_value_type(ValueTypeId::FontWeightAbsolute, component_values);
        if syntax_kind == CssValueTypeSyntaxKind::Invalid {
            return None;
        }

        let style_value = generated_value_type_style_value(syntax_kind, component_values);
        let source = serialize_component_values_for_reparsing(component_values, filtered_input)?;
        let (primitive_kind, numeric_value, source_or_unit) = match style_value.kind {
            GeneratedValueTypeStyleValueKind::Keyword => (
                CssPrimitiveValueKind::Keyword,
                None,
                style_value.value.unwrap_or(&source).to_string(),
            ),
            GeneratedValueTypeStyleValueKind::Number => {
                (CssPrimitiveValueKind::Number, style_value.numeric_value, source)
            }
            _ => (CssPrimitiveValueKind::Invalid, None, source),
        };

        self.index = start + 1;
        Some(RustOwnedDescriptorPrimitiveValue {
            primitive_kind,
            numeric_value,
            source_or_unit,
        })
    }

    fn consume_symbol_source(&mut self, filtered_input: &str) -> Option<String> {
        let start = self.index;
        if !self.consume_symbol_syntax() {
            return None;
        }
        serialize_component_values_for_reparsing(&self.component_values[start..self.index], filtered_input)
    }
}

pub(crate) fn parse_a_counter_style<C, S>(
    filtered_input: &[u8],
    mut counter_style_callback: C,
    mut symbol_callback: S,
) -> bool
where
    C: FnMut(CssCounterStyleKind, CssCounterStyleSymbolsType, Option<&str>),
    S: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(counter_style) = parser.parse_a_counter_style() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    match counter_style {
        CounterStyle::Name(name) => {
            counter_style_callback(
                CssCounterStyleKind::Name,
                CssCounterStyleSymbolsType::Symbolic,
                Some(&name),
            );
        }
        CounterStyle::SymbolsFunction { symbols_type, symbols } => {
            counter_style_callback(CssCounterStyleKind::SymbolsFunction, symbols_type, None);
            for symbol in &symbols {
                symbol_callback(symbol);
            }
        }
    }

    true
}

pub(crate) fn parse_a_nonnegative_integer_symbol_pair<O>(filtered_input: &[u8], mut order_callback: O) -> bool
where
    O: FnMut(CssNonnegativeIntegerSymbolPairOrder),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(order) = parser.parse_a_nonnegative_integer_symbol_pair() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    order_callback(order);
    true
}

pub(crate) fn parse_counter_style_negative<N>(filtered_input: &[u8], mut count_callback: N) -> bool
where
    N: FnMut(CssCounterStyleNegativeSymbolCount),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_negative() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_counter_style_system<S>(filtered_input: &[u8], mut system_callback: S) -> bool
where
    S: FnMut(CssCounterStyleSystemKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(system) = parser.parse_counter_style_system() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    system_callback(system);
    true
}

pub(crate) fn parse_counter_style_symbols<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_symbols() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_counter_style_symbol(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    if parser.parse_counter_style_symbol().is_none() {
        return false;
    }
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    true
}

pub(crate) fn parse_string_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/css-values-4/#strings
    // <string>
    component_values_parse_as_string(strip_whitespace(&component_values))
}

pub(crate) fn parse_length_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#lengths
    // <length>
    matches!(component_values, [component_value] if component_value_parse_as_length_descriptor(component_value))
}

pub(crate) fn parse_positive_percentage_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#percentages
    // <percentage [0,∞]>
    matches!(component_values, [component_value] if component_value_parse_as_positive_percentage_descriptor(component_value))
}

pub(crate) fn parse_page_size_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-page-3/#page-size-prop
    // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    if parser.parse_page_size_descriptor().is_none() {
        return false;
    }
    parser.discard_whitespace();
    !parser.has_next_component_value()
}

pub(crate) fn parse_optional_declaration_value_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-syntax/#typedef-declaration-value
    // The <declaration-value> production matches any sequence of one or more tokens, so long as the sequence does
    // not contain <bad-string-token>, <bad-url-token>, unmatched <)-token>, <]-token>, or <}-token>, or top-level
    // <semicolon-token> tokens or <delim-token> tokens with a value of "!". It represents the entirety of what a
    // valid declaration can have as its value.
    //
    // https://drafts.css-houdini.org/css-properties-values-api/#the-initial-value-descriptor
    // <declaration-value>?
    component_values.is_empty() || contains_only_declaration_value(component_values, Nested::No)
}

pub(crate) fn parse_counter_style_range<R>(filtered_input: &[u8], mut range_callback: R) -> bool
where
    R: FnMut(CssCounterStyleRangeKind, usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some((kind, count)) = parser.parse_counter_style_range() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    range_callback(kind, count);
    true
}

pub(crate) fn parse_counter_style_additive_symbols<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_additive_symbols() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_crop_or_cross<C>(filtered_input: &[u8], mut kind_callback: C) -> bool
where
    C: FnMut(CssCropOrCrossKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(kind) = parser.parse_crop_or_cross() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    kind_callback(kind);
    true
}

pub(crate) fn parse_font_weight_absolute_pair<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_font_weight_absolute_pair() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}
