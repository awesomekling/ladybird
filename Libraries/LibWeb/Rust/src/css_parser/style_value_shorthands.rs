/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(crate) fn parse_coordinating_value_list_shorthand<C>(
    property_ids: &[u16],
    filtered_input: &[u8],
    mut callback: C,
) -> bool
where
    C: FnMut(usize, u16, &str),
{
    let Some(items) = parse_rust_owned_coordinating_value_list_shorthand(property_ids, filtered_input) else {
        return false;
    };

    for item in items {
        callback(item.layer_index, item.style_value.property_id as u16, &item.source);
    }

    true
}

pub(crate) fn parse_rust_owned_coordinating_value_list_shorthand(
    property_ids: &[u16],
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedCoordinatingValueListShorthandItem>> {
    if property_ids.is_empty() {
        return None;
    }

    if !property_ids
        .iter()
        .all(|property_id| property_id_from_u16(*property_id).is_some())
    {
        return None;
    }

    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut layer_index = 0;
    let mut items = Vec::new();

    // https://drafts.csswg.org/css-values-4/#comb-comma
    // A hash mark (#) indicates that the preceding type, word, or group occurs
    // one or more times, separated by comma tokens (which may optionally be
    // surrounded by white space and/or comments).
    loop {
        let mut remaining_property_ids = property_ids.to_vec();
        let mut parsed_any_value = false;

        loop {
            parser.discard_whitespace();
            if matches!(
                parser.next_component_value(),
                None | Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                break;
            }

            let start = parser.index;
            parser.index += 1;
            let serialized_value = serialize_component_values_for_reparsing(
                &parser.component_values[start..parser.index],
                filtered_input_string,
            )?;

            let RustOwnedStyleValueParseResult::Parsed(style_value) =
                parse_rust_owned_style_value_for_property_with_mode(
                    &remaining_property_ids,
                    serialized_value.as_bytes(),
                    true,
                    CssPrimitiveValueOptions::default(),
                )
            else {
                return None;
            };
            let matched_property_id = style_value.property_id as u16;

            if !remaining_property_ids.contains(&matched_property_id) {
                return None;
            };

            remaining_property_ids.retain(|property_id| *property_id != matched_property_id);
            items.push(RustOwnedCoordinatingValueListShorthandItem {
                layer_index,
                style_value,
                source: serialized_value,
            });
            parsed_any_value = true;
        }

        if !parsed_any_value {
            return None;
        }

        parser.discard_whitespace();
        if !parser.consume_a_comma() {
            break;
        }

        layer_index += 1;
    }

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    if parsed_coordinating_value_list_shorthand_is_invalid(property_ids, &items) {
        return None;
    }

    Some(items)
}

fn parsed_coordinating_value_list_shorthand_is_invalid(
    property_ids: &[u16],
    items: &[RustOwnedCoordinatingValueListShorthandItem],
) -> bool {
    if is_transition_shorthand_property_list(property_ids) {
        return parsed_transition_shorthand_is_invalid(items);
    }

    false
}

fn is_transition_shorthand_property_list(property_ids: &[u16]) -> bool {
    let property_ids = property_ids
        .iter()
        .filter_map(|property_id| property_id_from_u16(*property_id))
        .collect::<Vec<_>>();

    property_ids.len() == 5
        && property_ids.contains(&PropertyId::TransitionProperty)
        && property_ids.contains(&PropertyId::TransitionDuration)
        && property_ids.contains(&PropertyId::TransitionTimingFunction)
        && property_ids.contains(&PropertyId::TransitionDelay)
        && property_ids.contains(&PropertyId::TransitionBehavior)
}

fn parsed_transition_shorthand_is_invalid(items: &[RustOwnedCoordinatingValueListShorthandItem]) -> bool {
    let parsed_layer_count = items
        .iter()
        .map(|item| item.layer_index)
        .max()
        .map_or(0, |layer_index| layer_index + 1);

    // https://drafts.csswg.org/css-transitions-1/#transition-shorthand-property
    // If there is more than one <single-transition> in the shorthand, and any
    // of the transitions has none as the <single-transition-property>, then the
    // declaration is invalid.
    parsed_layer_count > 1
        && items.iter().any(|item| {
            item.style_value.property_id == PropertyId::TransitionProperty
                && matches!(
                    &item.style_value.value,
                    RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(value))
                        if value.eq_ignore_ascii_case("none")
                )
        })
}

pub(crate) fn parse_layer_shorthand<C>(property_id: u16, filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(usize, u16, &str),
{
    let Some(items) = parse_rust_owned_layer_shorthand(property_id, filtered_input) else {
        return false;
    };

    for item in items {
        callback(item.layer_index, item.property_id as u16, &item.source);
    }

    true
}

pub(crate) fn parse_rust_owned_layer_shorthand(
    property_id: u16,
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedLayerShorthandItem>> {
    let property_id = property_id_from_u16(property_id)?;
    let longhand_property_ids = match property_id {
        PropertyId::Background => &[
            PropertyId::BackgroundAttachment,
            PropertyId::BackgroundClip,
            PropertyId::BackgroundColor,
            PropertyId::BackgroundImage,
            PropertyId::BackgroundOrigin,
            PropertyId::BackgroundPosition,
            PropertyId::BackgroundRepeat,
        ][..],
        PropertyId::Mask => &[
            PropertyId::MaskImage,
            PropertyId::MaskPosition,
            PropertyId::MaskRepeat,
            PropertyId::MaskOrigin,
            PropertyId::MaskClip,
            PropertyId::MaskComposite,
            PropertyId::MaskMode,
        ][..],
        _ => return None,
    };

    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let layers = parse_comma_separated_component_values(component_values, Some)?;

    if layers.is_empty() {
        return None;
    }

    let mut items = Vec::new();
    let last_layer_index = layers.len() - 1;
    for (layer_index, layer) in layers.into_iter().enumerate() {
        let layer_items = parse_layer_shorthand_layer(property_id, longhand_property_ids, layer_index, layer, &source)?;

        if layer_items.is_empty() {
            return None;
        }

        // https://drafts.csswg.org/css-backgrounds-3/#propdef-background
        // The <background-color> value may only be included in the last layer
        // specified.
        if property_id == PropertyId::Background
            && layer_index != last_layer_index
            && layer_items
                .iter()
                .any(|item| item.property_id == PropertyId::BackgroundColor)
        {
            return None;
        }

        items.extend(layer_items);
    }

    Some(items)
}

fn parse_layer_shorthand_layer(
    shorthand_property_id: PropertyId,
    longhand_property_ids: &[PropertyId],
    layer_index: usize,
    component_values: Vec<ComponentValue>,
    source: &str,
) -> Option<Vec<RustOwnedLayerShorthandItem>> {
    let mut remaining_property_ids = longhand_property_ids.to_vec();
    let mut items = Vec::new();
    let mut parser = ComponentValueParser::new(component_values);
    let mut parsed_size = false;

    loop {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }

        let item = consume_layer_shorthand_item(
            shorthand_property_id,
            &mut remaining_property_ids,
            &mut parser,
            layer_index,
            source,
            &mut parsed_size,
        )?;
        items.extend(item);
    }

    Some(items)
}

fn consume_layer_shorthand_item(
    shorthand_property_id: PropertyId,
    remaining_property_ids: &mut Vec<PropertyId>,
    parser: &mut ComponentValueParser,
    layer_index: usize,
    source: &str,
    parsed_size: &mut bool,
) -> Option<Vec<RustOwnedLayerShorthandItem>> {
    let start = parser.index;
    let mut candidate_property_ids = remaining_property_ids.clone();

    if shorthand_property_id == PropertyId::Background && *parsed_size {
        candidate_property_ids.retain(|property_id| *property_id != PropertyId::BackgroundPosition);
    } else if shorthand_property_id == PropertyId::Mask && *parsed_size {
        candidate_property_ids.retain(|property_id| *property_id != PropertyId::MaskPosition);
    }

    for property_id in candidate_property_ids {
        for end in ((start + 1)..=parser.component_values.len()).rev() {
            let candidate_source = serialize_component_values_for_reparsing(
                strip_whitespace(&parser.component_values[start..end]),
                source,
            )?;
            if !source_parses_as_property(property_id, &candidate_source) {
                continue;
            }

            parser.index = end;
            let mut items = vec![RustOwnedLayerShorthandItem {
                layer_index,
                property_id,
                source: candidate_source,
            }];
            remaining_property_ids.retain(|remaining_property_id| *remaining_property_id != property_id);

            if matches!(property_id, PropertyId::BackgroundPosition | PropertyId::MaskPosition) {
                parser.discard_whitespace();
                if parser.consume_a_delim('/') {
                    if *parsed_size {
                        return None;
                    }
                    parser.discard_whitespace();
                    let size_property_id = if property_id == PropertyId::BackgroundPosition {
                        PropertyId::BackgroundSize
                    } else {
                        PropertyId::MaskSize
                    };
                    let size_start = parser.index;
                    let size = consume_layer_shorthand_size(layer_index, size_property_id, parser, source, size_start)?;
                    items.push(size);
                    *parsed_size = true;
                }
            }

            return Some(items);
        }
    }

    None
}

fn consume_layer_shorthand_size(
    layer_index: usize,
    property_id: PropertyId,
    parser: &mut ComponentValueParser,
    source: &str,
    start: usize,
) -> Option<RustOwnedLayerShorthandItem> {
    for end in ((start + 1)..=parser.component_values.len()).rev() {
        let candidate_source =
            serialize_component_values_for_reparsing(strip_whitespace(&parser.component_values[start..end]), source)?;
        if !source_parses_as_property(property_id, &candidate_source) {
            continue;
        }

        parser.index = end;
        return Some(RustOwnedLayerShorthandItem {
            layer_index,
            property_id,
            source: candidate_source,
        });
    }

    None
}

fn source_parses_as_property(property_id: PropertyId, source: &str) -> bool {
    matches!(
        parse_rust_owned_style_value_for_property(&[property_id as u16], source.as_bytes()),
        RustOwnedStyleValueParseResult::Parsed(_)
    )
}

pub(crate) fn parse_grid_placement_shorthand<C>(property_id: u16, filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let Some(property_id) = property_id_from_u16(property_id) else {
        return false;
    };
    let Some(items) = parse_rust_owned_grid_placement_shorthand(property_id, filtered_input) else {
        return false;
    };

    for item in items {
        callback(item.property_id as u16, &item.source);
    }

    true
}

pub(super) fn parse_rust_owned_grid_placement_shorthand(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedGridPlacementShorthandItem>> {
    match property_id {
        PropertyId::GridColumn | PropertyId::GridRow => {
            parse_rust_owned_grid_track_placement_shorthand(property_id, filtered_input)
        }
        PropertyId::GridArea => parse_rust_owned_grid_area_shorthand(filtered_input),
        _ => None,
    }
}

fn parse_rust_owned_grid_track_placement_shorthand(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedGridPlacementShorthandItem>> {
    let (start_property, end_property) = match property_id {
        PropertyId::GridColumn => (PropertyId::GridColumnStart, PropertyId::GridColumnEnd),
        PropertyId::GridRow => (PropertyId::GridRowStart, PropertyId::GridRowEnd),
        _ => return None,
    };
    let source = filtered_input_to_string(filtered_input);
    let (mut stylesheet_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let parts = split_component_values_on_top_level_slashes(&component_values, &source)?;
    let ([start_source] | [start_source, _]) = parts.as_slice() else {
        return None;
    };
    let start_placement = parse_rust_owned_grid_track_placement_value(start_source.as_bytes())?;

    let end_source = if let Some(end_source) = parts.get(1) {
        parse_rust_owned_grid_track_placement_value(end_source.as_bytes())?;
        end_source.clone()
    } else if grid_track_placement_is_custom_ident(&start_placement) {
        start_source.clone()
    } else {
        "auto".to_string()
    };

    Some(vec![
        RustOwnedGridPlacementShorthandItem {
            property_id: start_property,
            source: start_source.clone(),
        },
        RustOwnedGridPlacementShorthandItem {
            property_id: end_property,
            source: end_source,
        },
    ])
}

fn parse_rust_owned_grid_area_shorthand(filtered_input: &[u8]) -> Option<Vec<RustOwnedGridPlacementShorthandItem>> {
    let source = filtered_input_to_string(filtered_input);
    let (mut stylesheet_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let parts = split_component_values_on_top_level_slashes(&component_values, &source)?;
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }

    let row_start = parse_rust_owned_grid_track_placement_value(parts[0].as_bytes())?;
    let column_start = if let Some(source) = parts.get(1) {
        Some(parse_rust_owned_grid_track_placement_value(source.as_bytes())?)
    } else {
        None
    };
    if let Some(source) = parts.get(2) {
        parse_rust_owned_grid_track_placement_value(source.as_bytes())?;
    }
    if let Some(source) = parts.get(3) {
        parse_rust_owned_grid_track_placement_value(source.as_bytes())?;
    }

    // https://www.w3.org/TR/css-grid-2/#placement-shorthands
    // If grid-column-start is omitted, if grid-row-start is a <custom-ident>,
    // all four longhands are set to that value. Otherwise, it is set to auto.
    let column_start_source = parts.get(1).cloned().unwrap_or_else(|| {
        if grid_track_placement_is_custom_ident(&row_start) {
            parts[0].clone()
        } else {
            "auto".to_string()
        }
    });

    // https://www.w3.org/TR/css-grid-2/#placement-shorthands
    // If grid-row-end is omitted, if grid-row-start is a <custom-ident>,
    // grid-row-end is set to that <custom-ident>; otherwise, it is set to auto.
    let row_end_source = parts.get(2).cloned().unwrap_or_else(|| {
        if grid_track_placement_is_custom_ident(&row_start) {
            parts[0].clone()
        } else {
            "auto".to_string()
        }
    });

    // https://www.w3.org/TR/css-grid-2/#placement-shorthands
    // If grid-column-end is omitted, if grid-column-start is a <custom-ident>,
    // grid-column-end is set to that <custom-ident>; otherwise, it is set to auto.
    let column_end_source = parts.get(3).cloned().unwrap_or_else(|| {
        let column_start_is_custom_ident = column_start
            .as_ref()
            .map(grid_track_placement_is_custom_ident)
            .unwrap_or_else(|| grid_track_placement_is_custom_ident(&row_start));
        if column_start_is_custom_ident {
            column_start_source.clone()
        } else {
            "auto".to_string()
        }
    });

    Some(vec![
        RustOwnedGridPlacementShorthandItem {
            property_id: PropertyId::GridRowStart,
            source: parts[0].clone(),
        },
        RustOwnedGridPlacementShorthandItem {
            property_id: PropertyId::GridColumnStart,
            source: column_start_source,
        },
        RustOwnedGridPlacementShorthandItem {
            property_id: PropertyId::GridRowEnd,
            source: row_end_source,
        },
        RustOwnedGridPlacementShorthandItem {
            property_id: PropertyId::GridColumnEnd,
            source: column_end_source,
        },
    ])
}

pub(crate) fn parse_grid_template_shorthand<C>(property_id: u16, filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let Some(property_id) = property_id_from_u16(property_id) else {
        return false;
    };
    let Some(items) = parse_rust_owned_grid_template_shorthand(property_id, filtered_input) else {
        return false;
    };

    for item in items {
        callback(item.property_id as u16, &item.source);
    }

    true
}

pub(super) fn parse_rust_owned_grid_template_shorthand(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedGridTemplateShorthandItem>> {
    let source = filtered_input_to_string(filtered_input);
    let (mut stylesheet_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let stripped_component_values = strip_whitespace(&component_values);

    match property_id {
        PropertyId::GridTemplate => {
            parse_rust_owned_grid_template_value(&component_values, stripped_component_values, &source)
        }
        PropertyId::Grid => parse_rust_owned_grid_value(&component_values, stripped_component_values, &source),
        _ => None,
    }
}

fn parse_rust_owned_grid_template_value(
    component_values: &[ComponentValue],
    stripped_component_values: &[ComponentValue],
    source: &str,
) -> Option<Vec<RustOwnedGridTemplateShorthandItem>> {
    // https://www.w3.org/TR/css-grid-2/#explicit-grid-shorthand
    // none | [ <'grid-template-rows'> / <'grid-template-columns'> ] |
    // [ <line-names>? <string> <track-size>? <line-names>? ]+
    // [ / <explicit-track-list> ]?
    if matches!(stripped_component_values, [component_value] if component_value_is_ident(Some(component_value), "none"))
    {
        return Some(vec![]);
    }

    if let Some(parts) = split_component_values_on_top_level_slashes(component_values, source)
        && let [rows_source, columns_source] = parts.as_slice()
        && parse_rust_owned_grid_track_size_list_value(rows_source.as_bytes(), GridTrackSizeListSyntax::TrackList)
            .is_some()
        && parse_rust_owned_grid_track_size_list_value(columns_source.as_bytes(), GridTrackSizeListSyntax::TrackList)
            .is_some()
    {
        return Some(vec![
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridTemplateRows,
                source: rows_source.clone(),
            },
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridTemplateColumns,
                source: columns_source.clone(),
            },
        ]);
    }

    parse_rust_owned_grid_template_areas_syntax(component_values, source)
}

fn parse_rust_owned_grid_value(
    component_values: &[ComponentValue],
    stripped_component_values: &[ComponentValue],
    source: &str,
) -> Option<Vec<RustOwnedGridTemplateShorthandItem>> {
    // https://www.w3.org/TR/css-grid-2/#grid-shorthand
    // <'grid-template'> |
    // <'grid-template-rows'> / [ auto-flow && dense? ] <'grid-auto-columns'>? |
    // [ auto-flow && dense? ] <'grid-auto-rows'>? / <'grid-template-columns'>
    if let Some(items) = parse_rust_owned_grid_template_value(component_values, stripped_component_values, source) {
        return Some(items);
    }

    let parts = split_component_values_on_top_level_slashes(component_values, source)?;
    let [left_source, right_source] = parts.as_slice() else {
        return None;
    };

    if let Some((grid_auto_flow_source, grid_auto_rows_source)) =
        parse_grid_auto_flow_prefix_with_optional_auto_track_sizes(left_source, CssGridAutoFlowAxis::Row)
        && parse_rust_owned_grid_track_size_list_value(right_source.as_bytes(), GridTrackSizeListSyntax::TrackList)
            .is_some()
    {
        let mut items = vec![
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridAutoFlow,
                source: grid_auto_flow_source,
            },
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridTemplateColumns,
                source: right_source.clone(),
            },
        ];
        if let Some(source) = grid_auto_rows_source {
            items.push(RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridAutoRows,
                source,
            });
        }
        return Some(items);
    }

    if parse_rust_owned_grid_track_size_list_value(left_source.as_bytes(), GridTrackSizeListSyntax::TrackList).is_some()
        && let Some((grid_auto_flow_source, grid_auto_columns_source)) =
            parse_grid_auto_flow_prefix_with_optional_auto_track_sizes(right_source, CssGridAutoFlowAxis::Column)
    {
        let mut items = vec![
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridTemplateRows,
                source: left_source.clone(),
            },
            RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridAutoFlow,
                source: grid_auto_flow_source,
            },
        ];
        if let Some(source) = grid_auto_columns_source {
            items.push(RustOwnedGridTemplateShorthandItem {
                property_id: PropertyId::GridAutoColumns,
                source,
            });
        }
        return Some(items);
    }

    None
}

fn parse_grid_auto_flow_prefix_with_optional_auto_track_sizes(
    input: &str,
    axis: CssGridAutoFlowAxis,
) -> Option<(String, Option<String>)> {
    let (mut stylesheet_parser, _) = parser_from_filtered_input(input.as_bytes());
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let mut found_auto_flow = false;
    let mut found_dense = false;

    for _ in 0..2 {
        parser.discard_whitespace();
        if component_value_is_ident(parser.next_component_value(), "auto-flow") && !found_auto_flow {
            found_auto_flow = true;
            parser.index += 1;
        } else if component_value_is_ident(parser.next_component_value(), "dense") && !found_dense {
            found_dense = true;
            parser.index += 1;
        } else {
            break;
        }
    }
    if !found_auto_flow {
        return None;
    }

    let grid_auto_flow_source = match (axis, found_dense) {
        (CssGridAutoFlowAxis::Row, false) => "row".to_string(),
        (CssGridAutoFlowAxis::Row, true) => "row dense".to_string(),
        (CssGridAutoFlowAxis::Column, false) => "column".to_string(),
        (CssGridAutoFlowAxis::Column, true) => "column dense".to_string(),
    };

    let auto_track_sizes_start = parser.index;
    let auto_track_sizes = parse_one_or_more_grid_track_sizes(&mut parser, input);
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    let grid_auto_track_sizes_source = if auto_track_sizes.is_some() {
        Some(serialize_component_values_for_reparsing(
            strip_whitespace(&parser.component_values[auto_track_sizes_start..parser.index]),
            input,
        )?)
    } else {
        None
    };

    Some((grid_auto_flow_source, grid_auto_track_sizes_source))
}

fn parse_rust_owned_grid_template_areas_syntax(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<Vec<RustOwnedGridTemplateShorthandItem>> {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let mut area_sources = Vec::new();
    let mut row_track_source_items = Vec::new();

    loop {
        parser.discard_whitespace();
        if component_value_is_delim(parser.next_component_value(), '/') || !parser.has_next_component_value() {
            break;
        }

        let line_names_before = parser.index;
        let leading_line_names = parse_grid_line_names(&mut parser);
        let line_names_after = parser.index;

        parser.discard_whitespace();
        let component_value = parser.next_component_value()?;
        if !matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { .. },
                ..
            })
        ) {
            return None;
        }
        area_sources.push(serialize_component_values_for_reparsing(
            std::slice::from_ref(component_value),
            source,
        )?);
        parser.index += 1;

        let track_size_start = parser.index;
        let track_size_source = if parse_grid_track_size(&mut parser, source).is_some() {
            serialize_component_values_for_reparsing(
                strip_whitespace(&parser.component_values[track_size_start..parser.index]),
                source,
            )?
        } else {
            "auto".to_string()
        };

        let trailing_line_names_before = parser.index;
        let trailing_line_names = parse_grid_line_names(&mut parser);
        let trailing_line_names_after = parser.index;

        serialize_component_values_for_reparsing(
            strip_whitespace(&parser.component_values[line_names_before..line_names_after]),
            source,
        )?;
        push_grid_template_line_names(&mut row_track_source_items, leading_line_names);
        row_track_source_items.push(GridTemplateRowTrackSourceItem::Track(track_size_source));
        serialize_component_values_for_reparsing(
            strip_whitespace(&parser.component_values[trailing_line_names_before..trailing_line_names_after]),
            source,
        )?;
        push_grid_template_line_names(&mut row_track_source_items, trailing_line_names);
    }

    if area_sources.is_empty() {
        return None;
    }

    let grid_template_areas_source = area_sources.join(" ");
    parse_rust_owned_grid_template_areas_value(grid_template_areas_source.as_bytes())?;
    let grid_template_rows_source = serialize_grid_template_row_track_source_items(&row_track_source_items);

    let mut items = vec![
        RustOwnedGridTemplateShorthandItem {
            property_id: PropertyId::GridTemplateAreas,
            source: grid_template_areas_source,
        },
        RustOwnedGridTemplateShorthandItem {
            property_id: PropertyId::GridTemplateRows,
            source: grid_template_rows_source,
        },
    ];

    parser.discard_whitespace();
    if component_value_is_delim(parser.next_component_value(), '/') {
        parser.index += 1;
        parser.discard_whitespace();
        let columns_start = parser.index;
        let columns = parse_grid_track_list(&mut parser, source)?;
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }
        if columns.is_empty() {
            return None;
        }
        items.push(RustOwnedGridTemplateShorthandItem {
            property_id: PropertyId::GridTemplateColumns,
            source: serialize_component_values_for_reparsing(
                strip_whitespace(&parser.component_values[columns_start..parser.index]),
                source,
            )?,
        });
    } else if parser.has_next_component_value() {
        return None;
    }

    Some(items)
}

pub(super) fn parse_rust_owned_grid_template_areas_value(filtered_input: &[u8]) -> Option<RustOwnedGridTemplateAreas> {
    let (mut stylesheet_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let stripped_component_values = strip_whitespace(&component_values);

    if matches!(stripped_component_values, [component_value] if component_value_is_ident(Some(component_value), "none"))
    {
        return Some(RustOwnedGridTemplateAreas::None);
    }

    let mut column_count = None;
    let mut grid_area_rows = Vec::new();
    for component_value in stripped_component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
    {
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        }) = component_value
        else {
            return None;
        };
        let row = parse_grid_template_area_string(value)?;
        if row.is_empty() {
            return None;
        }
        if let Some(column_count) = column_count {
            if row.len() != column_count {
                return None;
            }
        } else {
            column_count = Some(row.len());
        }
        grid_area_rows.push(row);
    }

    if grid_area_rows.is_empty() {
        return None;
    }

    validate_grid_template_areas_form_rectangles(&grid_area_rows)?;

    Some(RustOwnedGridTemplateAreas::Rows(grid_area_rows))
}

fn parse_grid_template_area_string(input: &str) -> Option<Vec<String>> {
    let mut row = Vec::new();
    for token in input.split_whitespace() {
        if token.chars().all(|character| character == '.') {
            row.push(".".to_string());
        } else if token.chars().all(is_grid_template_area_name_code_point) {
            row.push(token.to_string());
        } else {
            return None;
        }
    }
    Some(row)
}

fn is_grid_template_area_name_code_point(character: char) -> bool {
    // https://drafts.csswg.org/css-syntax-3/#ident-code-point
    // An ident-start code point, a digit, or U+002D HYPHEN-MINUS (-).
    character == '_' || character == '-' || character.is_ascii_alphanumeric() || !character.is_ascii()
}

fn validate_grid_template_areas_form_rectangles(grid_area_rows: &[Vec<String>]) -> Option<()> {
    use std::collections::HashMap;

    let mut name_counts = HashMap::new();
    for row in grid_area_rows {
        for cell in row {
            if cell != "." {
                *name_counts.entry(cell.as_str()).or_insert(0usize) += 1;
            }
        }
    }

    let mut grid_areas = HashMap::new();
    for (y, row) in grid_area_rows.iter().enumerate() {
        for (x, name) in row.iter().enumerate() {
            if name == "." || grid_areas.contains_key(name.as_str()) {
                continue;
            }

            let mut x_end = x;
            while x_end < row.len() && row[x_end] == *name {
                x_end += 1;
            }

            let mut y_end = y;
            while y_end < grid_area_rows.len() && grid_area_rows[y_end][x] == *name {
                y_end += 1;
            }

            let expected_count = (x_end - x) * (y_end - y);
            for check_row in grid_area_rows.iter().take(y_end).skip(y) {
                for cell in check_row.iter().take(x_end).skip(x) {
                    if cell != name {
                        return None;
                    }
                }
            }

            if name_counts.get(name.as_str()).copied().unwrap_or(0) != expected_count {
                return None;
            }

            grid_areas.insert(name.as_str(), (y, y_end, x, x_end));
        }
    }

    Some(())
}

fn push_grid_template_line_names(
    row_track_source_items: &mut Vec<GridTemplateRowTrackSourceItem>,
    line_names: Option<Vec<String>>,
) {
    let Some(line_names) = line_names else {
        return;
    };
    if line_names.is_empty() {
        return;
    }

    if let Some(GridTemplateRowTrackSourceItem::LineNames(previous_line_names)) = row_track_source_items.last_mut() {
        previous_line_names.extend(line_names);
    } else {
        row_track_source_items.push(GridTemplateRowTrackSourceItem::LineNames(line_names));
    }
}

fn serialize_grid_template_row_track_source_items(items: &[GridTemplateRowTrackSourceItem]) -> String {
    let mut sources = Vec::new();
    for item in items {
        match item {
            GridTemplateRowTrackSourceItem::LineNames(line_names) => {
                sources.push(format!("[{}]", line_names.join(" ")));
            }
            GridTemplateRowTrackSourceItem::Track(source) => sources.push(source.clone()),
        }
    }
    sources.join(" ")
}

fn split_component_values_on_top_level_slashes(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, component_value) in component_values.iter().enumerate() {
        if !matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Delim { value },
                ..
            }) if *value == '/' as u32
        ) {
            continue;
        }

        let part = serialize_component_values_for_reparsing(strip_whitespace(&component_values[start..index]), source)?;
        if part.is_empty() {
            return None;
        }
        parts.push(part);
        start = index + 1;
    }

    let part = serialize_component_values_for_reparsing(strip_whitespace(&component_values[start..]), source)?;
    if part.is_empty() {
        return None;
    }
    parts.push(part);
    Some(parts)
}

fn grid_track_placement_is_custom_ident(grid_track_placement: &RustOwnedGridTrackPlacement) -> bool {
    matches!(
        grid_track_placement,
        RustOwnedGridTrackPlacement::Line {
            line_number: None,
            name: Some(_),
        }
    )
}

pub(crate) fn parse_font_shorthand<C>(filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let Some(items) = parse_rust_owned_font_shorthand(filtered_input) else {
        return false;
    };

    for item in items {
        callback(item.property_id as u16, &item.source);
    }

    true
}

pub(super) fn parse_rust_owned_font_shorthand(filtered_input: &[u8]) -> Option<Vec<RustOwnedFontShorthandItem>> {
    let source = filtered_input_to_string(filtered_input);
    let (mut stylesheet_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = stylesheet_parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut items = Vec::new();
    let mut normal_count: u8 = 0;
    let mut parsed_font_style = false;
    let mut parsed_font_variant = false;
    let mut parsed_font_weight = false;
    let mut parsed_font_width = false;

    loop {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            return None;
        }

        if parser.next_component_value().is_some_and(|component_value| {
            matches!(
                component_value,
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                }) if value.eq_ignore_ascii_case("normal")
            )
        }) {
            normal_count += 1;
            parser.index += 1;
            continue;
        }

        // https://drafts.csswg.org/css-fonts-4/#font-prop
        // [ [ <'font-style'> || <font-variant-css2> || <'font-weight'> ||
        //     <font-width-css3> ]? <'font-size'> [ / <'line-height'> ]?
        //     <'font-family'># ] | <system-family-name>
        //
        // FIXME: Handle <system-family-name>. This matches the old C++ parser.
        if !parsed_font_variant
            && let Some(item) = consume_font_shorthand_item(&mut parser, PropertyId::FontVariant, &source, |source| {
                source.eq_ignore_ascii_case("small-caps")
            })
        {
            parsed_font_variant = true;
            items.push(item);
            continue;
        }

        if !parsed_font_width
            && let Some(item) = consume_font_shorthand_item(&mut parser, PropertyId::FontWidth, &source, |source| {
                !source.eq_ignore_ascii_case("normal")
                    && component_values_parse_as_generated_property_value_type(
                        ValueTypeId::FontWidthCss3,
                        source.as_bytes(),
                    )
            })
        {
            parsed_font_width = true;
            items.push(item);
            continue;
        }

        if let Some(final_items) = consume_font_size_line_height_and_family(&mut parser, &source) {
            items.extend(final_items);
            break;
        }

        if !parsed_font_style
            && let Some(item) = consume_font_shorthand_item(&mut parser, PropertyId::FontStyle, &source, |source| {
                source_parses_as_property(PropertyId::FontStyle, source)
            })
        {
            parsed_font_style = true;
            items.push(item);
            continue;
        }

        if !parsed_font_weight
            && let Some(item) = consume_font_shorthand_item(&mut parser, PropertyId::FontWeight, &source, |source| {
                source_parses_as_property(PropertyId::FontWeight, source)
            })
        {
            parsed_font_weight = true;
            items.push(item);
            continue;
        }

        return None;
    }

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    let unset_value_count = u8::from(!parsed_font_style)
        + u8::from(!parsed_font_variant)
        + u8::from(!parsed_font_weight)
        + u8::from(!parsed_font_width);
    if normal_count > unset_value_count {
        return None;
    }

    Some(items)
}

fn consume_font_shorthand_item<P>(
    parser: &mut ComponentValueParser,
    property_id: PropertyId,
    source: &str,
    predicate: P,
) -> Option<RustOwnedFontShorthandItem>
where
    P: Fn(&str) -> bool,
{
    let start = parser.index;
    for end in ((start + 1)..=parser.component_values.len()).rev() {
        let candidate_source =
            serialize_component_values_for_reparsing(strip_whitespace(&parser.component_values[start..end]), source)?;
        if !predicate(&candidate_source) {
            continue;
        }
        // AD-HOC: The Rust primitive validator accepts any function as an
        // angle because final materialization and range-checking still happen
        // in C++. In the `font` shorthand, that would make `oblique
        // calc(200 + 300) 24px Arial` consume the calc() as a font-style
        // angle instead of the following font-weight. Keep calc() with an
        // angle dimension on the font-style path, and leave other calc()
        // functions for the remaining shorthand slots.
        if property_id == PropertyId::FontStyle
            && font_style_shorthand_candidate_is_oblique_function_without_angle(strip_whitespace(
                &parser.component_values[start..end],
            ))
        {
            continue;
        }

        parser.index = end;
        return Some(RustOwnedFontShorthandItem {
            property_id,
            source: candidate_source,
        });
    }

    None
}

fn font_style_shorthand_candidate_is_oblique_function_without_angle(component_values: &[ComponentValue]) -> bool {
    let component_values: Vec<_> = component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .collect();
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
        ComponentValue::Function(function),
    ] = component_values.as_slice()
    else {
        return false;
    };

    value.eq_ignore_ascii_case("oblique") && !function_contains_dimension_type(function, DimensionType::Angle)
}

fn function_contains_dimension_type(function: &Function, dimension_type: DimensionType) -> bool {
    function.value.iter().any(|component_value| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => dimension_for_unit(unit) == Some(dimension_type),
        ComponentValue::Function(function) => function_contains_dimension_type(function, dimension_type),
        ComponentValue::SimpleBlock(block) => block.value.iter().any(|component_value| {
            matches!(
                component_value,
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Dimension { unit, .. },
                    ..
                }) if dimension_for_unit(unit) == Some(dimension_type)
            ) || matches!(
                component_value,
                ComponentValue::Function(function) if function_contains_dimension_type(function, dimension_type)
            )
        }),
        _ => false,
    })
}

fn consume_font_size_line_height_and_family(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<Vec<RustOwnedFontShorthandItem>> {
    let start = parser.index;
    for font_size_end in ((start + 1)..=parser.component_values.len()).rev() {
        let font_size_source = serialize_component_values_for_reparsing(
            strip_whitespace(&parser.component_values[start..font_size_end]),
            source,
        )?;
        if !source_parses_as_property(PropertyId::FontSize, &font_size_source) {
            continue;
        }
        let mut parser_after_font_size = ComponentValueParser::new(parser.component_values.clone());
        parser_after_font_size.index = font_size_end;
        parser_after_font_size.discard_whitespace();

        if parser_after_font_size.consume_a_delim('/') {
            parser_after_font_size.discard_whitespace();
            let line_height_start = parser_after_font_size.index;
            for line_height_end in ((line_height_start + 1)..=parser.component_values.len()).rev() {
                let line_height_source = serialize_component_values_for_reparsing(
                    strip_whitespace(&parser.component_values[line_height_start..line_height_end]),
                    source,
                )?;
                if !source_parses_as_property(PropertyId::LineHeight, &line_height_source) {
                    continue;
                }
                if let Some(font_family) = consume_font_family_after(parser, source, line_height_end) {
                    parser.index = parser.component_values.len();
                    return Some(vec![
                        RustOwnedFontShorthandItem {
                            property_id: PropertyId::FontSize,
                            source: font_size_source,
                        },
                        RustOwnedFontShorthandItem {
                            property_id: PropertyId::LineHeight,
                            source: line_height_source,
                        },
                        font_family,
                    ]);
                }
            }
            continue;
        }

        if let Some(font_family) = consume_font_family_after(parser, source, font_size_end) {
            parser.index = parser.component_values.len();
            return Some(vec![
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::FontSize,
                    source: font_size_source,
                },
                font_family,
            ]);
        }
    }

    None
}

fn consume_font_family_after(
    parser: &ComponentValueParser,
    source: &str,
    start: usize,
) -> Option<RustOwnedFontShorthandItem> {
    let family_source =
        serialize_component_values_for_reparsing(strip_whitespace(&parser.component_values[start..]), source)?;
    if family_source.is_empty() || !source_parses_as_property(PropertyId::FontFamily, &family_source) {
        return None;
    }

    Some(RustOwnedFontShorthandItem {
        property_id: PropertyId::FontFamily,
        source: family_source,
    })
}

pub(crate) fn parse_positional_value_list_shorthand<C>(property_id: u16, filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(usize, &str),
{
    let Some(items) = parse_rust_owned_positional_value_list_shorthand(
        property_id,
        filtered_input,
        CssPrimitiveValueOptions::default(),
    ) else {
        return false;
    };

    for item in items {
        callback(item.index, &item.source);
    }

    true
}

pub(crate) fn parse_rust_owned_positional_value_list_shorthand(
    property_id: u16,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<Vec<RustOwnedPositionalValueListShorthandItem>> {
    let property_id = property_id_from_u16(property_id)?;
    if !property_is_positional_value_list_shorthand(property_id) {
        return None;
    }

    let longhands = longhands_for_shorthand(property_id);
    if !matches!(longhands.len(), 2 | 4) {
        return None;
    }

    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut values = Vec::new();

    loop {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }
        if values.len() == longhands.len() {
            return None;
        }

        let start = parser.index;
        parser.index += 1;
        let serialized_value = serialize_component_values_for_reparsing(
            &parser.component_values[start..parser.index],
            filtered_input_string,
        )?;

        let property_ids = [longhands[values.len()] as u16];
        let RustOwnedStyleValueParseResult::Parsed(style_value) =
            parse_rust_owned_style_value_for_property_with_options(
                &property_ids,
                serialized_value.as_bytes(),
                primitive_value_options,
            )
        else {
            return None;
        };
        values.push(RustOwnedPositionalValueListShorthandItem {
            index: values.len(),
            style_value,
            source: serialized_value,
        });
    }

    if values.is_empty() {
        return None;
    }

    Some(values)
}
