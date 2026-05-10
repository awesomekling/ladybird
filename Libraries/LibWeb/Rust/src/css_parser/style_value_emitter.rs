/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

const COMPONENT_SHORTHAND_CALLBACK_ITEM_START: u8 = 255;
const POSITIONAL_VALUE_LIST_SHORTHAND_CALLBACK_ITEM_START: u8 = 255;
const SOURCE_COMPONENT_VALUE_LIST_FLEX_BASIS: u8 = 1;
const SOURCE_COMPONENT_VALUE_LIST_STYLE_COLOR: u8 = 2;
const SOURCE_COMPONENT_VALUE_LIST_IMAGE: u8 = 3;
const SOURCE_COMPONENT_VALUE_LIST_IMAGE_SET_RESOLUTION: u8 = 4;
const SOURCE_COMPONENT_VALUE_LIST_NESTED_PRIMITIVE: u8 = 5;
const SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM: u8 = 6;
const SOURCE_COMPONENT_VALUE_LIST_OPEN_TYPE_TAG_VALUE: u8 = 7;
const SOURCE_COMPONENT_VALUE_LIST_SECONDARY_NESTED_PRIMITIVE: u8 = 8;

struct SourceComponentValueEmitter<'a, S, E> {
    filtered_input: &'a str,
    list_callback: &'a mut S,
    component_value_callback: &'a mut E,
}

impl<S, E> SourceComponentValueEmitter<'_, S, E>
where
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    fn emit(&mut self, kind: u8, component_values: &[ComponentValue]) {
        (self.list_callback)(kind);
        emit_component_values(component_values, self.filtered_input, self.component_value_callback);
    }
}

fn emit_nested_primitive_source_component_values<S, E>(
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    value: &RustOwnedNestedPrimitiveValue,
) where
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedNestedPrimitiveValue::MathFunction(value) if !value.component_values.is_empty() => {
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_NESTED_PRIMITIVE, &value.component_values);
        }
        RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) if !value.component_values.is_empty() => {
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_NESTED_PRIMITIVE, &value.component_values);
        }
        RustOwnedNestedPrimitiveValue::Source { component_values, .. } if !component_values.is_empty() => {
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_NESTED_PRIMITIVE, component_values);
        }
        _ => {}
    }
}

fn emit_secondary_nested_primitive_source_component_values<S, E>(
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    value: &RustOwnedNestedPrimitiveValue,
) where
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedNestedPrimitiveValue::MathFunction(value) if !value.component_values.is_empty() => {
            source_component_value_emitter.emit(
                SOURCE_COMPONENT_VALUE_LIST_SECONDARY_NESTED_PRIMITIVE,
                &value.component_values,
            );
        }
        RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) if !value.component_values.is_empty() => {
            source_component_value_emitter.emit(
                SOURCE_COMPONENT_VALUE_LIST_SECONDARY_NESTED_PRIMITIVE,
                &value.component_values,
            );
        }
        RustOwnedNestedPrimitiveValue::Source { component_values, .. } if !component_values.is_empty() => {
            source_component_value_emitter
                .emit(SOURCE_COMPONENT_VALUE_LIST_SECONDARY_NESTED_PRIMITIVE, component_values);
        }
        _ => {}
    }
}

pub(super) fn emit_rust_owned_style_value<C>(style_value: &RustOwnedStyleValue, callback: &mut C)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    emit_rust_owned_style_value_with_calculation_callback(
        style_value,
        "",
        callback,
        &mut |_, _, _, _, _, _| {},
        &mut |_| {},
        &mut |_| {},
        &mut |_| {},
    );
}

pub(super) fn emit_rust_owned_style_value_with_calculation_callback<C, D, U, S, E>(
    style_value: &RustOwnedStyleValue,
    filtered_input: &str,
    callback: &mut C,
    calculation_callback: &mut D,
    url_modifier_callback: &mut U,
    source_component_value_list_callback: &mut S,
    source_component_value_callback: &mut E,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    U: FnMut(&UrlModifier),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let property_id = style_value.property_id as u16;
    match &style_value.value {
        RustOwnedStyleValueKind::Image(image) => callback_image_style_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            image,
        ),
        RustOwnedStyleValueKind::Anchor(_value) => {
            callback_source_backed_value_type_kind_style_value(
                callback,
                CssStyleValueKind::Anchor,
                property_id,
                "",
                PropertyValueType::Anchor,
            );
        }
        RustOwnedStyleValueKind::AnchorSize(value) => {
            callback_source_backed_value_type_kind_style_value(
                callback,
                CssStyleValueKind::AnchorSize,
                property_id,
                "",
                value.value_type,
            );
        }
        RustOwnedStyleValueKind::Counter(value) => {
            callback_counter_function_style_value(callback, CssStyleValueKind::Counter, property_id, value);
        }
        RustOwnedStyleValueKind::CornerShape(value) => {
            callback_corner_shape_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                &value.value,
            );
        }
        RustOwnedStyleValueKind::ImageSet(image_set) => callback_image_set_style_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            image_set,
        ),
        RustOwnedStyleValueKind::CoordinatingValueListShorthand(items) => {
            let shorthand_property_id = property_id;
            for item in items {
                callback(
                    CssStyleValueKind::CoordinatingValueListShorthand,
                    item.style_value.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    (item.layer_index & 0xff) as u8,
                    ((item.layer_index >> 8) & 0xff) as u8,
                    item.source.as_bytes(),
                    "",
                );
                SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                }
                .emit(SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM, &item.component_values);
                emit_rust_owned_style_value_with_calculation_callback(
                    &item.style_value,
                    &item.source,
                    callback,
                    calculation_callback,
                    url_modifier_callback,
                    source_component_value_list_callback,
                    source_component_value_callback,
                );
            }
        }
        RustOwnedStyleValueKind::FontShorthand(items) => {
            for item in items {
                callback(
                    CssStyleValueKind::FontShorthand,
                    item.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    0,
                    0,
                    0,
                    0,
                    item.source.as_bytes(),
                    "",
                );
                SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                }
                .emit(SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM, &item.component_values);
            }
        }
        RustOwnedStyleValueKind::ComponentShorthand(items) => {
            let shorthand_property_id = property_id;
            for item in items {
                callback(
                    CssStyleValueKind::ComponentShorthand,
                    item.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    COMPONENT_SHORTHAND_CALLBACK_ITEM_START,
                    0,
                    &[],
                    "",
                );
                SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                }
                .emit(SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM, &item.component_values);
                emit_rust_owned_style_value_with_calculation_callback(
                    &item.style_value,
                    &item.source,
                    callback,
                    calculation_callback,
                    url_modifier_callback,
                    source_component_value_list_callback,
                    source_component_value_callback,
                );
            }
        }
        RustOwnedStyleValueKind::GridPlacementShorthand(items) => {
            let shorthand_property_id = property_id;
            for item in items {
                callback_grid_placement_shorthand_item(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    shorthand_property_id,
                    item.property_id as u16,
                    &item.value,
                );
            }
        }
        RustOwnedStyleValueKind::GridTemplateShorthand(items) => {
            let shorthand_property_id = property_id;
            if items.is_empty() {
                callback(
                    CssStyleValueKind::GridTemplateShorthand,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    GRID_TEMPLATE_SHORTHAND_CALLBACK_EMPTY,
                    0,
                    &[],
                    "",
                );
                return;
            }
            for item in items {
                callback(
                    CssStyleValueKind::GridTemplateShorthand,
                    item.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    GRID_TEMPLATE_SHORTHAND_CALLBACK_ITEM_START,
                    0,
                    &[],
                    "",
                );
                match &item.style_value.value {
                    RustOwnedStyleValueKind::GridAutoFlow(value) => callback(
                        CssStyleValueKind::GridTemplateShorthand,
                        item.property_id as u16,
                        CssPrimitiveValueKind::Invalid,
                        false,
                        0.0,
                        false,
                        0.0,
                        value.axis as u8,
                        value.dense as u8,
                        0,
                        0,
                        &[],
                        "",
                    ),
                    RustOwnedStyleValueKind::GridTemplateAreas(value) => {
                        callback_grid_template_areas_style_value(
                            callback,
                            item.property_id as u16,
                            CssStyleValueKind::GridTemplateShorthand,
                            value,
                        );
                    }
                    RustOwnedStyleValueKind::GridTrackSizeList(value) => {
                        callback_grid_track_size_list_style_value(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            CssStyleValueKind::GridTemplateShorthand,
                            item.property_id as u16,
                            value,
                        );
                    }
                    RustOwnedStyleValueKind::GridAutoTrackSizes(value) => {
                        callback_grid_track_size_list_style_value(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            CssStyleValueKind::GridTemplateShorthand,
                            item.property_id as u16,
                            value,
                        );
                    }
                    _ => unreachable!("grid-template shorthand items are grid longhands"),
                }
            }
        }
        RustOwnedStyleValueKind::LayerShorthand(items) => {
            let shorthand_property_id = property_id;
            for item in items {
                callback(
                    CssStyleValueKind::LayerShorthand,
                    item.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    (item.layer_index & 0xff) as u8,
                    ((item.layer_index >> 8) & 0xff) as u8,
                    item.source.as_bytes(),
                    "",
                );
                SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                }
                .emit(SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM, &item.component_values);
                emit_rust_owned_style_value_with_calculation_callback(
                    &item.style_value,
                    &item.source,
                    callback,
                    calculation_callback,
                    url_modifier_callback,
                    source_component_value_list_callback,
                    source_component_value_callback,
                );
            }
        }
        RustOwnedStyleValueKind::PositionalValueListShorthand(items) => {
            let shorthand_property_id = property_id;
            for item in items {
                callback(
                    CssStyleValueKind::PositionalValueListShorthand,
                    item.style_value.property_id as u16,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    (shorthand_property_id & 0xff) as u8,
                    (shorthand_property_id >> 8) as u8,
                    (item.index & 0xff) as u8,
                    POSITIONAL_VALUE_LIST_SHORTHAND_CALLBACK_ITEM_START,
                    item.source.as_bytes(),
                    "",
                );
                SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                }
                .emit(SOURCE_COMPONENT_VALUE_LIST_SHORTHAND_ITEM, &item.component_values);
                emit_rust_owned_style_value_with_calculation_callback(
                    &item.style_value,
                    &item.source,
                    callback,
                    calculation_callback,
                    url_modifier_callback,
                    source_component_value_list_callback,
                    source_component_value_callback,
                );
            }
        }
        RustOwnedStyleValueKind::FontStyle(value) => {
            let (primitive_kind, numeric_value, unit_or_source) = value.angle.as_ref().map_or(
                (CssPrimitiveValueKind::Invalid, 0.0, ""),
                nested_primitive_callback_payload,
            );
            if let Some(angle) = &value.angle {
                emit_nested_primitive_source_component_values(
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    angle,
                );
            }
            callback(
                CssStyleValueKind::FontStyle,
                property_id,
                primitive_kind,
                value
                    .angle
                    .as_ref()
                    .is_some_and(nested_primitive_callback_has_numeric_value),
                numeric_value,
                false,
                0.0,
                css_font_style_kind(value.value) as u8,
                u8::from(value.angle.is_some()),
                0,
                0,
                unit_or_source.as_bytes(),
                property_value_type_name(PropertyValueType::FontStyle),
            );
        }
        RustOwnedStyleValueKind::AnchorNameOrScope(value) => {
            let name_bytes = null_separated_string_list_bytes(&value.names);
            callback(
                CssStyleValueKind::AnchorNameOrScope,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &name_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::AnimationName(value) => {
            let name_bytes = null_terminated_animation_name_item_bytes(&value.names);
            callback(
                CssStyleValueKind::AnimationName,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &name_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::AspectRatio(value) => {
            if value.numerator.is_none() {
                callback(
                    CssStyleValueKind::AspectRatio,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    2,
                    value.has_auto as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(numerator) = &value.numerator {
                callback_nested_primitive_with_source_component_values(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::AspectRatio,
                    property_id,
                    0,
                    0,
                    numerator,
                );
            }
            if value.has_auto && value.numerator.is_some() {
                callback(
                    CssStyleValueKind::AspectRatio,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    2,
                    1,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(denominator) = &value.denominator {
                callback_nested_primitive_with_source_component_values(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::AspectRatio,
                    property_id,
                    1,
                    0,
                    denominator,
                );
            }
        }
        RustOwnedStyleValueKind::BackgroundSize(value) => {
            for value in &value.values {
                callback_background_size(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    property_id,
                    value,
                );
            }
        }
        RustOwnedStyleValueKind::BorderRadius(value) => {
            for radius in &value.horizontal_radii {
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderRadius,
                    property_id,
                    0,
                    0,
                    radius,
                );
            }
            for radius in &value.vertical_radii {
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderRadius,
                    property_id,
                    1,
                    0,
                    radius,
                );
            }
        }
        RustOwnedStyleValueKind::BorderImageOutset(value) => {
            callback_border_image_outset_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::BorderImageOutset,
                property_id,
                &value.values,
            );
        }
        RustOwnedStyleValueKind::BorderImage(value) => {
            const SOURCE: u8 = 0;

            if let Some(source) = &value.source {
                let (kind, image_kind, url_function_type, payload) = match source {
                    RustOwnedBorderImageSource::None => (0, 0, IMAGE_URL_FUNCTION_TYPE_NONE, ""),
                    RustOwnedBorderImageSource::Image(image) => {
                        let (image_kind, url_function_type, payload) = image_callback_payload(image);
                        (1, image_kind, url_function_type, payload)
                    }
                };
                callback(
                    CssStyleValueKind::BorderImage,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    SOURCE,
                    kind,
                    image_kind,
                    url_function_type,
                    payload.as_bytes(),
                    "",
                );
                if let RustOwnedBorderImageSource::Image(image) = source
                    && !image.component_values.is_empty()
                {
                    SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    }
                    .emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.component_values);
                }
            }
            if let Some(slice) = &value.slice {
                callback_border_image_slice_style_value(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderImage,
                    property_id,
                    slice,
                );
            }
            if let Some(width) = &value.width {
                callback_border_image_width_style_value(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderImage,
                    property_id,
                    width,
                );
            }
            if let Some(outset) = &value.outset {
                callback_border_image_outset_style_value(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderImage,
                    property_id,
                    outset,
                );
            }
            if let Some(repeat) = &value.repeat {
                callback_border_image_repeat_style_value(callback, CssStyleValueKind::BorderImage, property_id, repeat);
            }
        }
        RustOwnedStyleValueKind::Border(value) => {
            const WIDTH: u8 = 0;
            const STYLE: u8 = 1;
            const COLOR: u8 = 2;

            if let Some(width) = &value.width {
                callback_border_width_style_value(
                    callback,
                    calculation_callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::Border,
                    property_id,
                    width,
                );
            }
            if let Some(style) = value.style {
                callback(
                    CssStyleValueKind::Border,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    STYLE,
                    style as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(color) = &value.color {
                callback_rust_owned_color(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::Border,
                    property_id,
                    COLOR,
                    color,
                );
            }
        }
        RustOwnedStyleValueKind::BorderImageRepeat(value) => {
            callback_border_image_repeat_style_value(
                callback,
                CssStyleValueKind::BorderImageRepeat,
                property_id,
                &value.values,
            );
        }
        RustOwnedStyleValueKind::BorderImageSlice(value) => {
            callback_border_image_slice_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::BorderImageSlice,
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::BorderImageWidth(value) => {
            callback_border_image_width_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::BorderImageWidth,
                property_id,
                &value.values,
            );
        }
        RustOwnedStyleValueKind::Columns(value) => {
            callback_optional_column_integer(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                0,
                value.column_count.as_ref(),
            );
            callback_optional_column_length(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                1,
                value.column_width.as_ref(),
            );
            callback_optional_column_length(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                2,
                value.column_height.as_ref(),
            );
        }
        RustOwnedStyleValueKind::Content(value) => callback_content_style_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            value,
        ),
        RustOwnedStyleValueKind::ColorScheme(value) => {
            let scheme_bytes = null_separated_string_list_bytes(&value.schemes);
            callback(
                CssStyleValueKind::ColorScheme,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.value.kind as u8,
                u8::from(value.value.only),
                0,
                0,
                &scheme_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::Contain(value) => callback(
            CssStyleValueKind::Contain,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            (value.value.is_size as u8)
                | ((value.value.is_inline_size as u8) << 1)
                | ((value.value.has_layout as u8) << 2)
                | ((value.value.has_style as u8) << 3)
                | ((value.value.has_paint as u8) << 4),
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::ContainerType(value) => callback(
            CssStyleValueKind::ContainerType,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value as u8,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::CounterDefinitions(value) => {
            for definition in &value.definitions {
                let (primitive_kind, has_numeric_value, numeric_value, source) = match &definition.value {
                    RustOwnedNestedPrimitiveValue::Integer(value) => {
                        (CssPrimitiveValueKind::Integer, true, *value as f64, "")
                    }
                    RustOwnedNestedPrimitiveValue::Source { source, .. } => {
                        (CssPrimitiveValueKind::Invalid, false, 0.0, source.as_str())
                    }
                    RustOwnedNestedPrimitiveValue::MathFunction(value) => {
                        (CssPrimitiveValueKind::Invalid, false, 0.0, value.source.as_str())
                    }
                    RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) => (
                        CssPrimitiveValueKind::Invalid,
                        false,
                        0.0,
                        match value.function {
                            RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count()",
                            RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index()",
                        },
                    ),
                    _ => unreachable!("counter definitions only use integer-like values"),
                };
                emit_nested_primitive_source_component_values(
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    &definition.value,
                );
                callback(
                    CssStyleValueKind::CounterDefinitions,
                    property_id,
                    primitive_kind,
                    has_numeric_value,
                    numeric_value,
                    false,
                    0.0,
                    u8::from(definition.is_reversed),
                    0,
                    0,
                    0,
                    definition.name.as_bytes(),
                    source,
                );
            }
        }
        RustOwnedStyleValueKind::Cursor(value) => callback_cursor_style_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            value,
        ),
        RustOwnedStyleValueKind::Display(value) => callback(
            CssStyleValueKind::Display,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.kind as u8,
            match value.kind {
                CssDisplayValueKind::Box => value.box_ as u8,
                CssDisplayValueKind::Internal => value.internal as u8,
                CssDisplayValueKind::OutsideAndInside => value.outside as u8,
                CssDisplayValueKind::Invalid => 0,
            },
            value.inside as u8,
            value.list_item as u8,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::None) => callback(
            CssStyleValueKind::Flex,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            FLEX_SHORTHAND_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::Longhands {
            flex_grow,
            flex_shrink,
            flex_basis,
        }) => {
            callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::Flex,
                property_id,
                FLEX_SHORTHAND_CALLBACK_GROW,
                0,
                flex_grow,
            );
            callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::Flex,
                property_id,
                FLEX_SHORTHAND_CALLBACK_SHRINK,
                0,
                flex_shrink,
            );
            callback_flex_basis(
                callback,
                filtered_input,
                source_component_value_list_callback,
                source_component_value_callback,
                property_id,
                flex_basis,
            );
        }
        RustOwnedStyleValueKind::FlexFlow(value) => {
            if let Some(flex_direction) = value.flex_direction {
                callback(
                    CssStyleValueKind::FlexFlow,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    0,
                    flex_direction as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(flex_wrap) = value.flex_wrap {
                callback(
                    CssStyleValueKind::FlexFlow,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    1,
                    flex_wrap as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
        }
        RustOwnedStyleValueKind::FilterValueList(value) => {
            callback_filter_value_list_style_value(
                callback,
                calculation_callback,
                url_modifier_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::GridAutoFlow(value) => callback(
            CssStyleValueKind::GridAutoFlow,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.axis as u8,
            value.dense as u8,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::GridAutoTrackSizes(value) => {
            callback_grid_track_size_list_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::GridAutoTrackSizes,
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::GridTemplateAreas(value) => {
            callback_grid_template_areas_style_value(
                callback,
                property_id,
                CssStyleValueKind::GridTemplateAreas,
                value,
            );
        }
        RustOwnedStyleValueKind::GridTrackPlacement(value) => {
            callback_grid_track_placement_style_value(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::GridTrackSizeList(value) => {
            callback_grid_track_size_list_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::GridTrackSizeList,
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::ListStyle(value) => {
            if let Some(position) = value.position {
                callback(
                    CssStyleValueKind::ListStyle,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    0,
                    position as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(image) = value.image.as_ref() {
                callback_list_style_image(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    property_id,
                    image,
                );
            }
            if let Some(list_style_type) = value.list_style_type.as_ref() {
                callback_list_style_type(callback, property_id, list_style_type);
            }
        }
        RustOwnedStyleValueKind::MathDepth(value) => match value {
            RustOwnedMathDepth::AutoAdd => callback(
                CssStyleValueKind::MathDepth,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                0,
                0,
                0,
                0,
                &[],
                "",
            ),
            RustOwnedMathDepth::Add { integer } => callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::MathDepth,
                property_id,
                1,
                0,
                integer,
            ),
            RustOwnedMathDepth::Integer { integer } => callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::MathDepth,
                property_id,
                2,
                0,
                integer,
            ),
        },
        RustOwnedStyleValueKind::Paint(value) => {
            callback_paint_style_value(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::PaintOrder(value) => callback(
            CssStyleValueKind::PaintOrder,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            value.value.first as u8,
            value.value.second as u8,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::PlaceContent(value) => {
            callback_place_shorthand_style_value(callback, CssStyleValueKind::PlaceContent, property_id, value);
        }
        RustOwnedStyleValueKind::PlaceItems(value) => {
            callback_place_shorthand_style_value(callback, CssStyleValueKind::PlaceItems, property_id, value);
        }
        RustOwnedStyleValueKind::PlaceSelf(value) => {
            callback_place_shorthand_style_value(callback, CssStyleValueKind::PlaceSelf, property_id, value);
        }
        RustOwnedStyleValueKind::PositionAnchor(value) => callback(
            CssStyleValueKind::PositionAnchor,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.kind as u8,
            0,
            0,
            0,
            value.name.as_ref().map_or(&[], |name| name.as_bytes()),
            "",
        ),
        RustOwnedStyleValueKind::PositionArea(value) => {
            callback_position_area_style_value(callback, CssStyleValueKind::PositionArea, property_id, value);
        }
        RustOwnedStyleValueKind::Position(value) => {
            callback_style_value_type(callback, CssStyleValueKind::Position, property_id, value.value_type);
            callback(
                CssStyleValueKind::Position,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                1,
                0,
                0,
                0,
                &[],
                "",
            );
            callback_position_component(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                2,
                &value.value.x,
            );
            callback_position_component(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                3,
                &value.value.y,
            );
        }
        RustOwnedStyleValueKind::PositionList(value) => {
            callback_style_value_type(callback, CssStyleValueKind::Position, property_id, value.value_type);
            for value in &value.values {
                match value {
                    RustOwnedPositionListItem::Position(position) => {
                        callback(
                            CssStyleValueKind::Position,
                            property_id,
                            CssPrimitiveValueKind::Invalid,
                            false,
                            0.0,
                            false,
                            0.0,
                            1,
                            0,
                            0,
                            0,
                            &[],
                            "",
                        );
                        callback_position_component(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            property_id,
                            2,
                            &position.x,
                        );
                        callback_position_component(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            property_id,
                            3,
                            &position.y,
                        );
                    }
                    RustOwnedPositionListItem::Component(component) => {
                        callback_position_component(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            property_id,
                            4,
                            component,
                        );
                    }
                }
            }
        }
        RustOwnedStyleValueKind::PositionTryFallbacks(value) => {
            callback_position_try_fallbacks_style_value(callback, property_id, value);
        }
        RustOwnedStyleValueKind::PositionTryOrder(value) => callback(
            CssStyleValueKind::PositionTryOrder,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value as u8,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::PositionVisibility(value) => callback(
            CssStyleValueKind::PositionVisibility,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            (value.value.has_anchors_valid as u8)
                | ((value.value.has_anchors_visible as u8) << 1)
                | ((value.value.has_no_overflow as u8) << 2),
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::Quotes(value) => {
            let string_bytes = null_separated_string_list_bytes(&value.strings);
            callback(
                CssStyleValueKind::Quotes,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &string_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::RepeatStyle(value) => {
            for value in &value.values {
                callback(
                    CssStyleValueKind::RepeatStyle,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    value.repeat_x as u8,
                    value.repeat_y as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
        }
        RustOwnedStyleValueKind::OverflowClipMargin(value) => {
            callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::OverflowClipMargin,
                property_id,
                0,
                0,
                &value.length,
            );
        }
        RustOwnedStyleValueKind::ScrollbarColor(value) => match value {
            RustOwnedScrollbarColor::Auto => callback(
                CssStyleValueKind::ScrollbarColor,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                1,
                0,
                0,
                0,
                &[],
                "",
            ),
            RustOwnedScrollbarColor::Colors {
                thumb_color,
                track_color,
            } => {
                callback_rust_owned_color(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::ScrollbarColor,
                    property_id,
                    2,
                    thumb_color,
                );
                callback_rust_owned_color(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::ScrollbarColor,
                    property_id,
                    3,
                    track_color,
                );
            }
        },
        RustOwnedStyleValueKind::ScrollbarGutter(value) => callback(
            CssStyleValueKind::ScrollbarGutter,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value as u8,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::Shadow(value) => {
            callback_shadow_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::ShapeOutside(value) => callback_shape_outside_style_value(
            callback,
            calculation_callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            value,
        ),
        RustOwnedStyleValueKind::TextDecoration(value) => {
            if let Some(line) = value.line {
                callback(
                    CssStyleValueKind::TextDecoration,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    0,
                    line.bits,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(thickness) = &value.thickness {
                callback_text_decoration_thickness_style_value(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    property_id,
                    thickness,
                );
            }
            if let Some(style) = value.style {
                callback(
                    CssStyleValueKind::TextDecoration,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    2,
                    style as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            if let Some(color) = &value.color {
                callback_rust_owned_color(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::TextDecoration,
                    property_id,
                    3,
                    color,
                );
            }
        }
        RustOwnedStyleValueKind::TextDecorationLine(value) => callback(
            CssStyleValueKind::TextDecorationLine,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.bits,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::ScrollTimeline(value) => {
            let name_bytes = null_terminated_timeline_name_item_bytes(&value.names);
            let axis_bytes: Vec<u8> = value.axes.iter().map(|axis| *axis as u8).collect();
            callback(
                CssStyleValueKind::ScrollTimeline,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                0,
                0,
                0,
                0,
                &name_bytes,
                std::str::from_utf8(&axis_bytes).unwrap(),
            );
        }
        RustOwnedStyleValueKind::TimelineName(value) => {
            let name_bytes = null_terminated_timeline_name_item_bytes(&value.names);
            callback(
                CssStyleValueKind::TimelineName,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &name_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::TimelineScope(value) => {
            let name_bytes = null_separated_string_list_bytes(&value.names);
            callback(
                CssStyleValueKind::TimelineScope,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &name_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::TextWrap(value) => callback(
            CssStyleValueKind::TextWrap,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.mode as u8,
            value.value.style as u8,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::TextWrapMode(value) => callback(
            CssStyleValueKind::TextWrapMode,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value as u8,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::TextWrapStyle(value) => callback(
            CssStyleValueKind::TextWrapStyle,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value as u8,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::TextIndent(value) => {
            let (primitive_kind, numeric_value, unit_or_source) =
                nested_primitive_callback_payload(&value.length_percentage);
            let value_type = match value.length_percentage {
                RustOwnedNestedPrimitiveValue::Percentage(_) => PropertyValueType::Percentage,
                _ => PropertyValueType::Length,
            };
            emit_nested_primitive_source_component_values(
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                &value.length_percentage,
            );
            callback(
                CssStyleValueKind::TextIndent,
                property_id,
                primitive_kind,
                nested_primitive_callback_has_numeric_value(&value.length_percentage),
                numeric_value,
                false,
                0.0,
                u8::from(value.has_hanging),
                u8::from(value.has_each_line),
                0,
                0,
                unit_or_source.as_bytes(),
                property_value_type_name(value_type),
            );
            if let RustOwnedNestedPrimitiveValue::MathFunction(value) = &value.length_percentage {
                emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
            }
        }
        RustOwnedStyleValueKind::TextUnderlinePosition(value) => callback(
            CssStyleValueKind::TextUnderlinePosition,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.horizontal as u8,
            value.value.vertical as u8,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::TransformOrigin(value) => {
            callback_transform_origin_component(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                0,
                &value.x,
            );
            callback_transform_origin_component(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                1,
                &value.y,
            );
            callback_nested_primitive_with_source_component_values_and_calculation(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::TransformOrigin,
                property_id,
                2,
                0,
                &value.z,
            );
        }
        RustOwnedStyleValueKind::TransformLonghand(value) => {
            callback_transform_longhand_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::Transformation(value) => {
            callback_transformation_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::TouchAction(value) => callback(
            CssStyleValueKind::TouchAction,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            value.value.first as u8,
            value.value.second as u8,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::TransitionBehavior(value) => {
            let behavior_bytes = value
                .behaviors
                .iter()
                .map(|behavior| *behavior as u8)
                .collect::<Vec<_>>();
            callback(
                CssStyleValueKind::TransitionBehavior,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &behavior_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::TransitionProperty(value) => {
            let mut property_bytes = Vec::new();
            for (index, property) in value.properties.iter().enumerate() {
                if index > 0 {
                    property_bytes.push(0);
                }
                property_bytes.extend_from_slice(property.as_bytes());
            }
            callback(
                CssStyleValueKind::TransitionProperty,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &property_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::ViewTransitionName(value) => callback(
            CssStyleValueKind::ViewTransitionName,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.kind as u8,
            0,
            0,
            0,
            value.name.as_ref().map_or(&[], |name| name.as_bytes()),
            "",
        ),
        RustOwnedStyleValueKind::WhiteSpace(value) => callback(
            CssStyleValueKind::WhiteSpace,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.text_wrap_mode as u8,
            value.white_space_trim.kind as u8,
            (value.white_space_trim.has_discard_before as u8)
                | ((value.white_space_trim.has_discard_after as u8) << 1)
                | ((value.white_space_trim.has_discard_inner as u8) << 2),
            0,
            value.white_space_collapse.as_bytes(),
            "",
        ),
        RustOwnedStyleValueKind::WhiteSpaceTrim(value) => callback(
            CssStyleValueKind::WhiteSpaceTrim,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            (value.value.has_discard_before as u8)
                | ((value.value.has_discard_after as u8) << 1)
                | ((value.value.has_discard_inner as u8) << 2),
            0,
            0,
            &[],
            "",
        ),
        RustOwnedStyleValueKind::WillChange(value) => {
            let feature_bytes = null_terminated_will_change_feature_bytes(&value.features);
            callback(
                CssStyleValueKind::WillChange,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                value.kind as u8,
                0,
                0,
                0,
                &feature_bytes,
                "",
            );
        }
        RustOwnedStyleValueKind::GeneratedValueList(value) => {
            for item in &value.items {
                callback(
                    CssStyleValueKind::GeneratedValueList,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    0,
                    0,
                    0,
                    0,
                    &[],
                    property_value_type_name(item.value_type),
                );
            }
        }
        RustOwnedStyleValueKind::Primitive(value) => callback_rust_owned_primitive_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            value,
        ),
        RustOwnedStyleValueKind::Identifier(value) => {
            callback_rust_owned_identifier_value(callback, property_id, value);
        }
        RustOwnedStyleValueKind::StrokeDasharray(value) => match value {
            RustOwnedStrokeDasharray::None => callback(
                CssStyleValueKind::StrokeDasharray,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                1,
                0,
                0,
                0,
                &[],
                "",
            ),
            RustOwnedStrokeDasharray::Values(values) => {
                for value in values {
                    callback_nested_primitive_with_source_component_values_and_calculation(
                        callback,
                        calculation_callback,
                        &mut SourceComponentValueEmitter {
                            filtered_input,
                            list_callback: source_component_value_list_callback,
                            component_value_callback: source_component_value_callback,
                        },
                        CssStyleValueKind::StrokeDasharray,
                        property_id,
                        0,
                        0,
                        value,
                    );
                }
            }
        },
        RustOwnedStyleValueKind::BorderSpacing(value) => {
            for value in &value.values {
                callback_nested_primitive_with_source_component_values(
                    callback,
                    &mut SourceComponentValueEmitter {
                        filtered_input,
                        list_callback: source_component_value_list_callback,
                        component_value_callback: source_component_value_callback,
                    },
                    CssStyleValueKind::BorderSpacing,
                    property_id,
                    0,
                    0,
                    value,
                );
            }
        }
        RustOwnedStyleValueKind::MathFunction(value) => {
            callback_rust_owned_math_function(callback, property_id, value);
        }
        RustOwnedStyleValueKind::TreeCountingFunction(value) => {
            callback_rust_owned_tree_counting_function(callback, property_id, value);
        }
        RustOwnedStyleValueKind::CounterStyle(value) => {
            callback_counter_style(callback, CssStyleValueKind::CounterStyle, property_id, value);
        }
        RustOwnedStyleValueKind::FontVariantLonghand(value) => {
            callback_font_variant_longhand_style_value(callback, property_id, value);
        }
        RustOwnedStyleValueKind::KeywordList(value) => {
            callback_keyword_list_style_value(callback, property_id, value);
        }
        RustOwnedStyleValueKind::Shorthand(value_list)
        | RustOwnedStyleValueKind::Tuple(value_list)
        | RustOwnedStyleValueKind::ValueList(value_list) => {
            if value_list.value_type == Some(PropertyValueType::TransformList) {
                for value in &value_list.values {
                    if let RustOwnedStyleValueKind::Transformation(transformation) = value {
                        callback_transformation_style_value(
                            callback,
                            calculation_callback,
                            &mut SourceComponentValueEmitter {
                                filtered_input,
                                list_callback: source_component_value_list_callback,
                                component_value_callback: source_component_value_callback,
                            },
                            property_id,
                            transformation,
                        );
                    }
                }
                return;
            }
            let _ = value_list;
        }
        RustOwnedStyleValueKind::GuaranteedInvalid => {}
        RustOwnedStyleValueKind::Color(color) => callback_color_style_value(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            property_id,
            color,
        ),
        RustOwnedStyleValueKind::Url(value) => callback_url_style_value(callback, property_id, value),
        RustOwnedStyleValueKind::EasingFunction(value) => {
            callback_easing_function_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::FitContent(value) => {
            callback_fit_content_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                &value.value,
            );
        }
        RustOwnedStyleValueKind::FontFamily(value) => {
            for value in &value.values {
                let (kind, family_name, is_string) = match value {
                    FontFamilyValue::Generic(value) => (CssFontFamilyValueKind::Generic, value, false),
                    FontFamilyValue::FamilyName(value) => {
                        (CssFontFamilyValueKind::FamilyName, &value.name, value.is_string)
                    }
                };
                callback(
                    CssStyleValueKind::FontFamily,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    kind as u8,
                    u8::from(is_string),
                    0,
                    0,
                    family_name.as_bytes(),
                    "",
                );
            }
        }
        RustOwnedStyleValueKind::OpenTypeSettings(value) => {
            callback_open_type_settings_style_value(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::FontLanguageOverride(value) => callback(
            CssStyleValueKind::FontLanguageOverride,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.kind as u8,
            0,
            0,
            0,
            value.value.as_ref().map_or(&[], |value| value.as_bytes()),
            "",
        ),
        RustOwnedStyleValueKind::FontVariant(value) => {
            callback_font_variant_style_value(callback, property_id, value);
        }
        RustOwnedStyleValueKind::BasicShape(value) => {
            callback_basic_shape_style_value(
                callback,
                calculation_callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::Rect(value) => {
            callback_rect_style_value(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                property_id,
                value,
            );
        }
        RustOwnedStyleValueKind::ScrollFunction(value) => callback(
            CssStyleValueKind::ScrollFunction,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.scroller as u8,
            value.axis as u8,
            0,
            0,
            &[],
            property_value_type_name(PropertyValueType::ScrollFunction),
        ),
        RustOwnedStyleValueKind::ViewTimeline(value) => {
            let name_bytes = null_terminated_timeline_name_item_bytes(&value.names);
            callback(
                CssStyleValueKind::ViewTimeline,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                0,
                0,
                0,
                0,
                &name_bytes,
                &String::from_utf8(value.axes.iter().map(|axis| *axis as u8).collect()).unwrap(),
            );
            for inset in &value.insets {
                callback_view_timeline_inset_count(callback, CssStyleValueKind::ViewTimeline, property_id, inset.len());
                for value in inset {
                    callback_view_timeline_inset_value(
                        callback,
                        &mut SourceComponentValueEmitter {
                            filtered_input,
                            list_callback: source_component_value_list_callback,
                            component_value_callback: source_component_value_callback,
                        },
                        CssStyleValueKind::ViewTimeline,
                        property_id,
                        value,
                    );
                }
            }
        }
        RustOwnedStyleValueKind::ViewTimelineInset(value) => {
            for inset in &value.insets {
                callback_view_timeline_inset_count(
                    callback,
                    CssStyleValueKind::ViewTimelineInset,
                    property_id,
                    inset.len(),
                );
                for value in inset {
                    callback_view_timeline_inset_value(
                        callback,
                        &mut SourceComponentValueEmitter {
                            filtered_input,
                            list_callback: source_component_value_list_callback,
                            component_value_callback: source_component_value_callback,
                        },
                        CssStyleValueKind::ViewTimelineInset,
                        property_id,
                        value,
                    );
                }
            }
        }
        RustOwnedStyleValueKind::ViewFunction(value) => callback(
            CssStyleValueKind::ViewFunction,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.axis as u8,
            value.inset as u8,
            value.inset_position as u8,
            0,
            &[],
            property_value_type_name(PropertyValueType::ViewFunction),
        ),
    }
}

fn callback_primitive_style_value<C>(
    callback: &mut C,
    property_id: u16,
    primitive_kind: CssPrimitiveValueKind,
    numeric_value: Option<f64>,
    secondary_numeric_value: Option<f64>,
    value: &[u8],
    value_type: PropertyValueType,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::Primitive,
        property_id,
        primitive_kind,
        numeric_value.is_some(),
        numeric_value.unwrap_or(0.0),
        secondary_numeric_value.is_some(),
        secondary_numeric_value.unwrap_or(0.0),
        0,
        0,
        0,
        0,
        value,
        property_value_type_name(value_type),
    );
}

fn callback_rust_owned_primitive_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedPrimitiveValue::Nested { value, value_type } => {
            let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);
            emit_nested_primitive_source_component_values(source_component_value_emitter, value);
            callback_primitive_style_value(
                callback,
                property_id,
                primitive_kind,
                nested_primitive_callback_has_numeric_value(value).then_some(numeric_value),
                None,
                unit_or_source.as_bytes(),
                *value_type,
            );
        }
        RustOwnedPrimitiveValue::Ratio {
            numerator,
            denominator,
            has_denominator,
            value_type,
        } => callback_primitive_style_value(
            callback,
            property_id,
            CssPrimitiveValueKind::Ratio,
            Some(*numerator),
            Some(*denominator),
            if *has_denominator { b"has-denominator" } else { b"" },
            *value_type,
        ),
        RustOwnedPrimitiveValue::Token {
            primitive_kind,
            numeric_value,
            secondary_numeric_value,
            value,
            value_type,
        } => callback_primitive_style_value(
            callback,
            property_id,
            *primitive_kind,
            *numeric_value,
            *secondary_numeric_value,
            value.as_bytes(),
            *value_type,
        ),
    }
}

fn callback_rust_owned_identifier_value<C>(callback: &mut C, property_id: u16, value: &RustOwnedIdentifierValue)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedIdentifierValue::Keyword(value) => callback(
            CssStyleValueKind::Keyword,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            TRANSFORM_LONGHAND_CALLBACK_NONE,
            0,
            0,
            0,
            value.as_bytes(),
            "",
        ),
        RustOwnedIdentifierValue::CustomIdent { value, value_type } => callback(
            CssStyleValueKind::CustomIdent,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            value.as_bytes(),
            property_value_type_name(*value_type),
        ),
        RustOwnedIdentifierValue::CounterStyleName(value) => callback(
            CssStyleValueKind::CounterStyleName,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            value.as_bytes(),
            property_value_type_name(PropertyValueType::CounterStyle),
        ),
    }
}

fn callback_rust_owned_math_function<C>(callback: &mut C, property_id: u16, value: &RustOwnedMathFunction)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback_source_backed_value_type_kind_style_value(
        callback,
        CssStyleValueKind::MathFunction,
        property_id,
        &value.source,
        value.value_type,
    );
}

fn callback_rust_owned_tree_counting_function<C>(
    callback: &mut C,
    property_id: u16,
    value: &RustOwnedTreeCountingFunction,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::TreeCountingFunction,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        value.function as u8,
        0,
        0,
        0,
        &[],
        property_value_type_name(value.value_type),
    );
}

fn callback_style_value_type<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    value_type: PropertyValueType,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        0,
        0,
        0,
        0,
        &[],
        property_value_type_name(value_type),
    );
}

fn callback_open_type_settings_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedOpenTypeSettingsStyleValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let kind = match value.kind {
        RustOwnedOpenTypeSettingsStyleValueKind::FontFeatureSettings => CssStyleValueKind::FontFeatureSettings,
        RustOwnedOpenTypeSettingsStyleValueKind::FontVariationSettings => CssStyleValueKind::FontVariationSettings,
    };
    callback(
        kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        value.value.kind as u8,
        0,
        0,
        0,
        &[],
        "",
    );

    for tag_value in &value.value.tag_values {
        let mut tag_and_value = tag_value.tag.as_bytes().to_vec();
        if let Some(value) = &tag_value.value {
            tag_and_value.extend_from_slice(value.as_bytes());
        }
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            value.value.kind as u8,
            tag_value.value_kind as u8,
            0,
            0,
            &tag_and_value,
            "",
        );
        if !tag_value.value_component_values.is_empty() {
            source_component_value_emitter.emit(
                SOURCE_COMPONENT_VALUE_LIST_OPEN_TYPE_TAG_VALUE,
                &tag_value.value_component_values,
            );
        }
    }
}

fn css_font_style_kind(value: FontStyle) -> CssFontStyleKind {
    match value {
        FontStyle::Normal => CssFontStyleKind::Normal,
        FontStyle::Italic => CssFontStyleKind::Italic,
        FontStyle::Left => CssFontStyleKind::Left,
        FontStyle::Right => CssFontStyleKind::Right,
        FontStyle::Oblique { .. } => CssFontStyleKind::Oblique,
    }
}

fn callback_source_backed_style_value<C>(callback: &mut C, kind: CssStyleValueKind, property_id: u16, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        0,
        0,
        0,
        0,
        source.as_bytes(),
        "",
    );
}

const EASING_FUNCTION_CALLBACK_KEYWORD: u8 = 0;
const EASING_FUNCTION_CALLBACK_LINEAR: u8 = 1;
const EASING_FUNCTION_CALLBACK_CUBIC_BEZIER: u8 = 2;
const EASING_FUNCTION_CALLBACK_STEPS: u8 = 3;

const BASIC_SHAPE_CALLBACK_INSET: u8 = 0;
const BASIC_SHAPE_CALLBACK_XYWH: u8 = 1;
const BASIC_SHAPE_CALLBACK_RECT: u8 = 2;
const BASIC_SHAPE_CALLBACK_CIRCLE: u8 = 3;
const BASIC_SHAPE_CALLBACK_ELLIPSE: u8 = 4;
const BASIC_SHAPE_CALLBACK_POLYGON: u8 = 5;
const BASIC_SHAPE_CALLBACK_PATH: u8 = 6;

const BASIC_SHAPE_COMPONENT_HEADER: u8 = 0;
const BASIC_SHAPE_COMPONENT_POLYGON_POINT_X: u8 = 1;
const BASIC_SHAPE_COMPONENT_POLYGON_POINT_Y: u8 = 2;
const BASIC_SHAPE_COMPONENT_RECTANGLE_LENGTH_PERCENTAGE: u8 = 3;
const BASIC_SHAPE_COMPONENT_RECTANGLE_AUTO: u8 = 4;
const BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_HORIZONTAL: u8 = 5;
const BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_VERTICAL: u8 = 6;
const BASIC_SHAPE_COMPONENT_RADIAL_EXTENT: u8 = 7;
const BASIC_SHAPE_COMPONENT_RADIAL_LENGTH_PERCENTAGE: u8 = 8;
const BASIC_SHAPE_COMPONENT_RADIAL_POSITION_X: u8 = 9;
const BASIC_SHAPE_COMPONENT_RADIAL_POSITION_Y: u8 = 10;

const FIT_CONTENT_CALLBACK_KEYWORD: u8 = 0;
const FIT_CONTENT_CALLBACK_FUNCTION: u8 = 1;

fn callback_easing_function_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedEasingFunction,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    const LINEAR_OUTPUT: u8 = 0;
    const LINEAR_FIRST_STOP_LENGTH: u8 = 1;
    const LINEAR_SECOND_STOP_LENGTH: u8 = 2;

    match &value.value {
        RustOwnedEasingFunctionValue::Keyword(keyword) => callback(
            CssStyleValueKind::EasingFunction,
            property_id,
            CssPrimitiveValueKind::Keyword,
            false,
            0.0,
            false,
            0.0,
            EASING_FUNCTION_CALLBACK_KEYWORD,
            0,
            1,
            0,
            keyword.as_bytes(),
            "",
        ),
        RustOwnedEasingFunctionValue::Linear(stops) => {
            for stop in stops {
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    CssStyleValueKind::EasingFunction,
                    property_id,
                    EASING_FUNCTION_CALLBACK_LINEAR,
                    LINEAR_OUTPUT,
                    &stop.output,
                );
                if let Some(first_stop_length) = &stop.first_stop_length {
                    callback_nested_primitive_with_source_component_values_and_calculation(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        CssStyleValueKind::EasingFunction,
                        property_id,
                        EASING_FUNCTION_CALLBACK_LINEAR,
                        LINEAR_FIRST_STOP_LENGTH,
                        first_stop_length,
                    );
                }
                if let Some(second_stop_length) = &stop.second_stop_length {
                    callback_nested_primitive_with_source_component_values_and_calculation(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        CssStyleValueKind::EasingFunction,
                        property_id,
                        EASING_FUNCTION_CALLBACK_LINEAR,
                        LINEAR_SECOND_STOP_LENGTH,
                        second_stop_length,
                    );
                }
            }
        }
        RustOwnedEasingFunctionValue::CubicBezier { x1, y1, x2, y2 } => {
            for (index, value) in [x1, y1, x2, y2].iter().enumerate() {
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    CssStyleValueKind::EasingFunction,
                    property_id,
                    EASING_FUNCTION_CALLBACK_CUBIC_BEZIER,
                    index as u8,
                    value,
                );
            }
        }
        RustOwnedEasingFunctionValue::Steps { intervals, position } => {
            callback_nested_primitive_with_source_component_values_and_calculation(
                callback,
                calculation_callback,
                source_component_value_emitter,
                CssStyleValueKind::EasingFunction,
                property_id,
                EASING_FUNCTION_CALLBACK_STEPS,
                position.map(rust_owned_step_position_callback_payload).unwrap_or(5),
                intervals,
            );
        }
    }
}

fn rust_owned_step_position_callback_payload(position: RustOwnedStepPosition) -> u8 {
    match position {
        RustOwnedStepPosition::JumpStart => 0,
        RustOwnedStepPosition::JumpEnd => 1,
        RustOwnedStepPosition::JumpNone => 2,
        RustOwnedStepPosition::JumpBoth => 3,
        RustOwnedStepPosition::Start => 4,
        RustOwnedStepPosition::End => 5,
    }
}

fn callback_border_image_slice_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedBorderImageSlice,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for slice_value in &value.values {
        callback_nested_primitive_with_source_component_values_and_calculation(
            callback,
            calculation_callback,
            source_component_value_emitter,
            kind,
            property_id,
            1,
            value.fill.into(),
            slice_value,
        );
    }
}

fn callback_border_width_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    if let RustOwnedNestedPrimitiveValue::Keyword(keyword) = value {
        let Some(line_width) = line_width_from_keyword(keyword) else {
            unreachable!("border-width keywords are validated while parsing")
        };
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            line_width,
            0,
            0,
            &[],
            "",
        );
    } else {
        callback_nested_primitive_with_source_component_values_and_calculation(
            callback,
            calculation_callback,
            source_component_value_emitter,
            kind,
            property_id,
            0,
            1,
            value,
        );
    }
}

fn line_width_from_keyword(keyword: &str) -> Option<u8> {
    if keyword == "thin" {
        return Some(LINE_WIDTH_THIN);
    }
    if keyword == "medium" {
        return Some(LINE_WIDTH_MEDIUM);
    }
    if keyword == "thick" {
        return Some(LINE_WIDTH_THICK);
    }
    None
}

fn callback_rust_owned_color<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    component_kind: u8,
    color: &RustOwnedColor,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match color {
        RustOwnedColor::Simple {
            kind: color_kind,
            red,
            green,
            blue,
            alpha,
            name,
        } => callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            true,
            f64::from(component_kind),
            true,
            *color_kind as u8 as f64,
            *red,
            *green,
            *blue,
            *alpha,
            name.as_deref().unwrap_or("").as_bytes(),
            "",
        ),
        RustOwnedColor::Function {
            source,
            component_values,
            ..
        } => {
            callback(
                kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                component_kind,
                0,
                0,
                0,
                source.as_bytes(),
                "",
            );
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_STYLE_COLOR, component_values);
        }
    }
}

fn callback_color_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    color: &RustOwnedColor,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match color {
        RustOwnedColor::Simple {
            red,
            green,
            blue,
            alpha,
            name,
            ..
        } => callback(
            CssStyleValueKind::Color,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            *red,
            *green,
            *blue,
            *alpha,
            name.as_deref().unwrap_or("").as_bytes(),
            "",
        ),
        RustOwnedColor::Function {
            source,
            component_values,
            ..
        } => {
            callback_source_backed_value_type_kind_style_value(
                callback,
                CssStyleValueKind::ColorFunction,
                property_id,
                source,
                PropertyValueType::Color,
            );
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_STYLE_COLOR, component_values);
        }
    }
}

fn callback_border_image_width_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    values: &[RustOwnedNestedPrimitiveValue],
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for value in values {
        match value {
            RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => callback(
                kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                2,
                1,
                0,
                0,
                &[],
                "",
            ),
            _ => {
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    kind,
                    property_id,
                    2,
                    0,
                    value,
                );
            }
        }
    }
}

fn callback_border_image_outset_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    values: &[RustOwnedBorderImageOutset],
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for value in values {
        callback_nested_primitive_with_source_component_values_and_calculation(
            callback,
            calculation_callback,
            source_component_value_emitter,
            kind,
            property_id,
            3,
            0,
            &value.value,
        );
    }
}

fn callback_border_image_repeat_style_value<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    values: &[RustOwnedBorderImageRepeat],
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    for value in values {
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            4,
            *value as u8,
            0,
            0,
            &[],
            "",
        );
    }
}

fn callback_fit_content_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    if matches!(value, RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "fit-content") {
        callback(
            CssStyleValueKind::FitContent,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            FIT_CONTENT_CALLBACK_KEYWORD,
            0,
            0,
            0,
            b"",
            "",
        );
    } else {
        callback_nested_primitive_with_source_component_values_and_calculation(
            callback,
            calculation_callback,
            source_component_value_emitter,
            CssStyleValueKind::FitContent,
            property_id,
            FIT_CONTENT_CALLBACK_FUNCTION,
            0,
            value,
        );
    }
}

fn callback_text_decoration_thickness_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => callback(
            CssStyleValueKind::TextDecoration,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            1,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "from-font" => callback(
            CssStyleValueKind::TextDecoration,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            1,
            1,
            0,
            0,
            &[],
            "",
        ),
        _ => {
            callback_nested_primitive_with_source_component_values(
                callback,
                source_component_value_emitter,
                CssStyleValueKind::TextDecoration,
                property_id,
                1,
                2,
                value,
            );
        }
    }
}

fn callback_basic_shape_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (kind, path_data) = basic_shape_callback_payload(value);

    if matches!(
        value.kind,
        RustOwnedBasicShapeKind::Inset | RustOwnedBasicShapeKind::Xywh | RustOwnedBasicShapeKind::Rect
    ) {
        callback_basic_shape_header(
            callback,
            CssStyleValueKind::BasicShape,
            property_id,
            kind,
            value.fill_rule,
        );
        callback_basic_shape_rectangle_components(
            callback,
            calculation_callback,
            source_component_value_emitter,
            CssStyleValueKind::BasicShape,
            property_id,
            kind,
            value,
        );
        return;
    }

    if matches!(
        value.kind,
        RustOwnedBasicShapeKind::Circle | RustOwnedBasicShapeKind::Ellipse
    ) {
        callback_basic_shape_header(
            callback,
            CssStyleValueKind::BasicShape,
            property_id,
            kind,
            value.fill_rule,
        );
        callback_basic_shape_radial_components(
            callback,
            calculation_callback,
            source_component_value_emitter,
            CssStyleValueKind::BasicShape,
            property_id,
            kind,
            value,
        );
        return;
    }

    if value.kind == RustOwnedBasicShapeKind::Polygon {
        callback_basic_shape_header(
            callback,
            CssStyleValueKind::BasicShape,
            property_id,
            kind,
            value.fill_rule,
        );
        for point in &value.polygon_points {
            callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                CssStyleValueKind::BasicShape,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: value.fill_rule,
                    component: BASIC_SHAPE_COMPONENT_POLYGON_POINT_X,
                },
                &point.x,
            );
            callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                CssStyleValueKind::BasicShape,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: value.fill_rule,
                    component: BASIC_SHAPE_COMPONENT_POLYGON_POINT_Y,
                },
                &point.y,
            );
        }
        return;
    }

    callback(
        CssStyleValueKind::BasicShape,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        value.fill_rule as u8,
        0,
        0,
        path_data.as_bytes(),
        "",
    );
}

fn callback_basic_shape_radial_components<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for component in &value.radial_shape_radius {
        if let RustOwnedNestedPrimitiveValue::Keyword(keyword) = component {
            let Some(extent) = radial_extent_from_keyword(keyword) else {
                unreachable!("radial shape radius keywords are validated while parsing")
            };
            callback(
                style_value_kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                kind,
                0,
                BASIC_SHAPE_COMPONENT_RADIAL_EXTENT,
                extent as u8,
                &[],
                "",
            );
        } else {
            callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                style_value_kind,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    component: BASIC_SHAPE_COMPONENT_RADIAL_LENGTH_PERCENTAGE,
                },
                component,
            );
        }
    }

    if let Some(position) = &value.radial_shape_position {
        callback_basic_shape_position_component(
            callback,
            calculation_callback,
            source_component_value_emitter,
            BasicShapePositionComponentCallback {
                style_value_kind,
                property_id,
                kind,
                component_kind: BASIC_SHAPE_COMPONENT_RADIAL_POSITION_X,
            },
            &position.x,
        );
        callback_basic_shape_position_component(
            callback,
            calculation_callback,
            source_component_value_emitter,
            BasicShapePositionComponentCallback {
                style_value_kind,
                property_id,
                kind,
                component_kind: BASIC_SHAPE_COMPONENT_RADIAL_POSITION_Y,
            },
            &position.y,
        );
    }
}

struct BasicShapePositionComponentCallback {
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    component_kind: u8,
}

fn callback_basic_shape_position_component<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    payload: BasicShapePositionComponentCallback,
    component: &RustOwnedPositionComponent,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let edge = component.edge.map_or(0, rust_owned_position_edge_to_callback_value);
    let Some(offset) = component.offset.as_ref() else {
        callback(
            payload.style_value_kind,
            payload.property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            payload.kind,
            edge,
            payload.component_kind,
            0,
            &[],
            "",
        );
        return;
    };

    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(offset);
    emit_nested_primitive_source_component_values(source_component_value_emitter, offset);
    callback(
        payload.style_value_kind,
        payload.property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(offset),
        numeric_value,
        false,
        0.0,
        payload.kind,
        edge,
        payload.component_kind,
        1,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = offset {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn callback_basic_shape_rectangle_components<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for component in &value.rectangle_components {
        match component {
            RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => callback(
                style_value_kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                kind,
                0,
                BASIC_SHAPE_COMPONENT_RECTANGLE_AUTO,
                0,
                &[],
                "",
            ),
            _ => callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                style_value_kind,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    component: BASIC_SHAPE_COMPONENT_RECTANGLE_LENGTH_PERCENTAGE,
                },
                component,
            ),
        }
    }

    if let Some(border_radius) = &value.rectangle_border_radius {
        for radius in &border_radius.horizontal_radii {
            callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                style_value_kind,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    component: BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_HORIZONTAL,
                },
                radius,
            );
        }
        for radius in &border_radius.vertical_radii {
            callback_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                style_value_kind,
                property_id,
                BasicShapeNestedPrimitiveCallback {
                    kind,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    component: BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_VERTICAL,
                },
                radius,
            );
        }
    }
}

fn callback_basic_shape_header<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    fill_rule: RustOwnedBasicShapeFillRule,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        style_value_kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        fill_rule as u8,
        BASIC_SHAPE_COMPONENT_HEADER,
        0,
        &[],
        "",
    );
}

struct BasicShapeNestedPrimitiveCallback {
    kind: u8,
    fill_rule: RustOwnedBasicShapeFillRule,
    component: u8,
}

fn callback_basic_shape_nested_primitive<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    payload: BasicShapeNestedPrimitiveCallback,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);

    emit_nested_primitive_source_component_values(source_component_value_emitter, value);
    callback(
        style_value_kind,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(value),
        numeric_value,
        false,
        0.0,
        payload.kind,
        payload.fill_rule as u8,
        payload.component,
        0,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = value {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn basic_shape_callback_payload(value: &RustOwnedBasicShape) -> (u8, String) {
    let kind = match value.kind {
        RustOwnedBasicShapeKind::Inset => BASIC_SHAPE_CALLBACK_INSET,
        RustOwnedBasicShapeKind::Xywh => BASIC_SHAPE_CALLBACK_XYWH,
        RustOwnedBasicShapeKind::Rect => BASIC_SHAPE_CALLBACK_RECT,
        RustOwnedBasicShapeKind::Circle => BASIC_SHAPE_CALLBACK_CIRCLE,
        RustOwnedBasicShapeKind::Ellipse => BASIC_SHAPE_CALLBACK_ELLIPSE,
        RustOwnedBasicShapeKind::Polygon => BASIC_SHAPE_CALLBACK_POLYGON,
        RustOwnedBasicShapeKind::Path => BASIC_SHAPE_CALLBACK_PATH,
    };
    let payload = value.path_data.clone().unwrap_or_default();
    (kind, payload)
}

fn callback_rect_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedRect,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for side in &value.sides {
        callback_nested_primitive_with_source_component_values(
            callback,
            source_component_value_emitter,
            CssStyleValueKind::Rect,
            property_id,
            u8::from(value.requires_commas),
            0,
            side,
        );
    }
}

fn callback_source_backed_value_type_kind_style_value<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    source: &str,
    value_type: PropertyValueType,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        0,
        0,
        0,
        0,
        source.as_bytes(),
        property_value_type_name(value_type),
    );
}

fn callback_optional_longhand_source<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    index: u8,
    source: Option<&String>,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    if let Some(source) = source {
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            index,
            0,
            0,
            0,
            source.as_bytes(),
            "",
        );
    }
}

const GRID_TRACK_SIZE_LIST_CALLBACK_NONE: u8 = 0;
const GRID_TRACK_SIZE_LIST_CALLBACK_LINE_NAMES: u8 = 1;
const GRID_TRACK_SIZE_LIST_CALLBACK_BREADTH: u8 = 2;
const GRID_TRACK_SIZE_LIST_CALLBACK_MINMAX: u8 = 3;
const GRID_TRACK_SIZE_LIST_CALLBACK_FIT_CONTENT: u8 = 4;
const GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_BEGIN: u8 = 5;
const GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_END: u8 = 6;
const GRID_TRACK_SIZE_LIST_CALLBACK_SECONDARY_CALCULATION_TARGET: u8 = 7;
const GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_AUTO_FILL: u8 = 0;
const GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_AUTO_FIT: u8 = 1;
const GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_FIXED: u8 = 2;
const GRID_TRACK_BREADTH_INVALID: u8 = 0;
const GRID_TRACK_BREADTH_LENGTH_PERCENTAGE: u8 = 1;
const GRID_TRACK_BREADTH_FLEX: u8 = 2;
const GRID_TRACK_BREADTH_MIN_CONTENT: u8 = 3;
const GRID_TRACK_BREADTH_MAX_CONTENT: u8 = 4;
const GRID_TRACK_BREADTH_AUTO: u8 = 5;

const GRID_TEMPLATE_SHORTHAND_CALLBACK_EMPTY: u8 = 254;
const GRID_TEMPLATE_SHORTHAND_CALLBACK_ITEM_START: u8 = 255;

const GRID_TEMPLATE_AREAS_CALLBACK_NONE: u8 = 0;
const GRID_TEMPLATE_AREAS_CALLBACK_ROW: u8 = 1;

fn callback_grid_template_areas_style_value<C>(
    callback: &mut C,
    property_id: u16,
    kind: CssStyleValueKind,
    value: &RustOwnedGridTemplateAreas,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedGridTemplateAreas::None => callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            GRID_TEMPLATE_AREAS_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedGridTemplateAreas::Rows(rows) => {
            for row in rows {
                let row = null_separated_string_list_bytes(row);
                callback(
                    kind,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    GRID_TEMPLATE_AREAS_CALLBACK_ROW,
                    0,
                    0,
                    0,
                    &row,
                    "",
                );
            }
        }
    }
}

fn callback_grid_track_size_list_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedGridTrackSizeList,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedGridTrackSizeList::None => callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            GRID_TRACK_SIZE_LIST_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedGridTrackSizeList::List(items) => {
            callback_grid_track_size_list_items(
                callback,
                calculation_callback,
                source_component_value_emitter,
                kind,
                property_id,
                items,
            );
        }
    }
}

fn callback_grid_track_size_list_items<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    items: &[RustOwnedGridTrackSizeListItem],
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for item in items {
        match item {
            RustOwnedGridTrackSizeListItem::LineNames(names) => {
                let names = null_separated_string_list_bytes(names);
                callback(
                    kind,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    GRID_TRACK_SIZE_LIST_CALLBACK_LINE_NAMES,
                    0,
                    0,
                    0,
                    &names,
                    "",
                );
            }
            RustOwnedGridTrackSizeListItem::Track(track) => {
                callback_explicit_grid_track(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    kind,
                    property_id,
                    track,
                );
            }
        }
    }
}

fn callback_explicit_grid_track<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    track: &RustOwnedExplicitGridTrack,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match track {
        RustOwnedExplicitGridTrack::Size(size) => match size {
            RustOwnedGridTrackSize::Breadth(breadth) => {
                let payload = grid_track_breadth_callback_payload(breadth);
                emit_nested_primitive_source_component_values(source_component_value_emitter, breadth);
                callback(
                    kind,
                    property_id,
                    payload.primitive_kind,
                    payload.has_numeric_value,
                    payload.numeric_value,
                    false,
                    0.0,
                    GRID_TRACK_SIZE_LIST_CALLBACK_BREADTH,
                    0,
                    payload.breadth_kind,
                    0,
                    payload.source_or_unit.as_bytes(),
                    "",
                );
                emit_grid_track_nested_primitive_calculation(calculation_callback, breadth);
            }
            RustOwnedGridTrackSize::MinMax { min, max } => {
                let min_payload = grid_track_breadth_callback_payload(min);
                let max_payload = grid_track_breadth_callback_payload(max);
                emit_nested_primitive_source_component_values(source_component_value_emitter, min);
                emit_secondary_nested_primitive_source_component_values(source_component_value_emitter, max);
                callback(
                    kind,
                    property_id,
                    min_payload.primitive_kind,
                    min_payload.has_numeric_value,
                    min_payload.numeric_value,
                    max_payload.has_numeric_value,
                    max_payload.numeric_value,
                    GRID_TRACK_SIZE_LIST_CALLBACK_MINMAX,
                    max_payload.breadth_kind,
                    min_payload.breadth_kind,
                    max_payload.primitive_kind as u8,
                    min_payload.source_or_unit.as_bytes(),
                    max_payload.source_or_unit,
                );
                emit_grid_track_nested_primitive_calculation(calculation_callback, min);
                callback_grid_track_secondary_calculation_target(callback, kind, property_id);
                emit_grid_track_nested_primitive_calculation(calculation_callback, max);
            }
            RustOwnedGridTrackSize::FitContent(value) => {
                let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);
                emit_nested_primitive_source_component_values(source_component_value_emitter, value);
                callback(
                    kind,
                    property_id,
                    primitive_kind,
                    nested_primitive_callback_has_numeric_value(value),
                    numeric_value,
                    false,
                    0.0,
                    GRID_TRACK_SIZE_LIST_CALLBACK_FIT_CONTENT,
                    0,
                    GRID_TRACK_BREADTH_LENGTH_PERCENTAGE,
                    0,
                    unit_or_source.as_bytes(),
                    "",
                );
                emit_grid_track_nested_primitive_calculation(calculation_callback, value);
            }
        },
        RustOwnedExplicitGridTrack::Repeat(repeat) => {
            let (repeat_type, count) = match &repeat.repeat_type {
                RustOwnedGridRepeatType::AutoFill => (GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_AUTO_FILL, None),
                RustOwnedGridRepeatType::AutoFit => (GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_AUTO_FIT, None),
                RustOwnedGridRepeatType::Fixed { count } => (GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_FIXED, Some(count)),
            };
            let (primitive_kind, has_numeric_value, numeric_value, source_or_unit) = if let Some(count) = count {
                let (primitive_kind, numeric_value, source_or_unit) = nested_primitive_callback_payload(count);
                emit_nested_primitive_source_component_values(source_component_value_emitter, count);
                (
                    primitive_kind,
                    nested_primitive_callback_has_numeric_value(count),
                    numeric_value,
                    source_or_unit,
                )
            } else {
                (CssPrimitiveValueKind::Invalid, false, 0.0, "")
            };

            callback(
                kind,
                property_id,
                primitive_kind,
                has_numeric_value,
                numeric_value,
                false,
                0.0,
                GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_BEGIN,
                repeat_type,
                0,
                0,
                source_or_unit.as_bytes(),
                "",
            );
            if let Some(count) = count {
                emit_grid_track_nested_primitive_calculation(calculation_callback, count);
            }
            callback_grid_track_size_list_items(
                callback,
                calculation_callback,
                source_component_value_emitter,
                kind,
                property_id,
                &repeat.track_list,
            );
            callback(
                kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                GRID_TRACK_SIZE_LIST_CALLBACK_REPEAT_END,
                0,
                0,
                0,
                &[],
                "",
            );
        }
    }
}

fn callback_grid_track_secondary_calculation_target<C>(callback: &mut C, kind: CssStyleValueKind, property_id: u16)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        GRID_TRACK_SIZE_LIST_CALLBACK_SECONDARY_CALCULATION_TARGET,
        0,
        0,
        0,
        &[],
        "",
    );
}

fn emit_grid_track_nested_primitive_calculation<D>(calculation_callback: &mut D, value: &RustOwnedNestedPrimitiveValue)
where
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
{
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = value {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

struct GridTrackBreadthCallbackPayload<'a> {
    breadth_kind: u8,
    primitive_kind: CssPrimitiveValueKind,
    has_numeric_value: bool,
    numeric_value: f64,
    source_or_unit: &'a str,
}

fn grid_track_breadth_callback_payload(breadth: &RustOwnedNestedPrimitiveValue) -> GridTrackBreadthCallbackPayload<'_> {
    match breadth {
        RustOwnedNestedPrimitiveValue::Number(_)
        | RustOwnedNestedPrimitiveValue::Length { .. }
        | RustOwnedNestedPrimitiveValue::Percentage(_)
        | RustOwnedNestedPrimitiveValue::Source { .. } => {
            let (primitive_kind, numeric_value, source_or_unit) = nested_primitive_callback_payload(breadth);
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_LENGTH_PERCENTAGE,
                primitive_kind,
                has_numeric_value: nested_primitive_callback_has_numeric_value(breadth),
                numeric_value,
                source_or_unit,
            }
        }
        RustOwnedNestedPrimitiveValue::MathFunction(value) if value.value_type != PropertyValueType::Flex => {
            let (primitive_kind, numeric_value, source_or_unit) = nested_primitive_callback_payload(breadth);
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_LENGTH_PERCENTAGE,
                primitive_kind,
                has_numeric_value: nested_primitive_callback_has_numeric_value(breadth),
                numeric_value,
                source_or_unit,
            }
        }
        RustOwnedNestedPrimitiveValue::Flex { .. } => {
            let (primitive_kind, numeric_value, source_or_unit) = nested_primitive_callback_payload(breadth);
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_FLEX,
                primitive_kind,
                has_numeric_value: nested_primitive_callback_has_numeric_value(breadth),
                numeric_value,
                source_or_unit,
            }
        }
        RustOwnedNestedPrimitiveValue::MathFunction(_) => {
            let (primitive_kind, numeric_value, source_or_unit) = nested_primitive_callback_payload(breadth);
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_FLEX,
                primitive_kind,
                has_numeric_value: nested_primitive_callback_has_numeric_value(breadth),
                numeric_value,
                source_or_unit,
            }
        }
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "min-content" => {
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_MIN_CONTENT,
                primitive_kind: CssPrimitiveValueKind::Invalid,
                has_numeric_value: false,
                numeric_value: 0.0,
                source_or_unit: "",
            }
        }
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "max-content" => {
            GridTrackBreadthCallbackPayload {
                breadth_kind: GRID_TRACK_BREADTH_MAX_CONTENT,
                primitive_kind: CssPrimitiveValueKind::Invalid,
                has_numeric_value: false,
                numeric_value: 0.0,
                source_or_unit: "",
            }
        }
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => GridTrackBreadthCallbackPayload {
            breadth_kind: GRID_TRACK_BREADTH_AUTO,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            has_numeric_value: false,
            numeric_value: 0.0,
            source_or_unit: "",
        },
        _ => {
            unreachable!("grid track breadths only use length-percentage, flex, source-backed math, and known keywords")
        }
    }
}

const GRID_TRACK_PLACEMENT_CALLBACK_AUTO: u8 = 0;
const GRID_TRACK_PLACEMENT_CALLBACK_LINE: u8 = 1;
const GRID_TRACK_PLACEMENT_CALLBACK_SPAN: u8 = 2;

fn callback_grid_track_placement_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedGridTrackPlacement,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (kind, line_number, name) = match value {
        RustOwnedGridTrackPlacement::Auto => (GRID_TRACK_PLACEMENT_CALLBACK_AUTO, None, None),
        RustOwnedGridTrackPlacement::Line { line_number, name } => (
            GRID_TRACK_PLACEMENT_CALLBACK_LINE,
            line_number.as_ref(),
            name.as_deref(),
        ),
        RustOwnedGridTrackPlacement::Span { line_number, name } => (
            GRID_TRACK_PLACEMENT_CALLBACK_SPAN,
            line_number.as_ref(),
            name.as_deref(),
        ),
    };
    let (primitive_kind, numeric_value, source_or_unit) = line_number
        .map(nested_primitive_callback_payload)
        .unwrap_or((CssPrimitiveValueKind::Invalid, 0.0, ""));
    if let Some(line_number) = line_number {
        emit_nested_primitive_source_component_values(source_component_value_emitter, line_number);
    }

    callback(
        CssStyleValueKind::GridTrackPlacement,
        property_id,
        primitive_kind,
        line_number.is_some() && primitive_kind != CssPrimitiveValueKind::Invalid,
        numeric_value,
        false,
        0.0,
        kind,
        0,
        0,
        0,
        source_or_unit.as_bytes(),
        name.unwrap_or(""),
    );
}

fn callback_grid_placement_shorthand_item<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    shorthand_property_id: u16,
    property_id: u16,
    value: &RustOwnedGridTrackPlacement,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (kind, line_number, name) = match value {
        RustOwnedGridTrackPlacement::Auto => (GRID_TRACK_PLACEMENT_CALLBACK_AUTO, None, None),
        RustOwnedGridTrackPlacement::Line { line_number, name } => (
            GRID_TRACK_PLACEMENT_CALLBACK_LINE,
            line_number.as_ref(),
            name.as_deref(),
        ),
        RustOwnedGridTrackPlacement::Span { line_number, name } => (
            GRID_TRACK_PLACEMENT_CALLBACK_SPAN,
            line_number.as_ref(),
            name.as_deref(),
        ),
    };
    let (primitive_kind, numeric_value, source_or_unit) = line_number
        .map(nested_primitive_callback_payload)
        .unwrap_or((CssPrimitiveValueKind::Invalid, 0.0, ""));
    if let Some(line_number) = line_number {
        emit_nested_primitive_source_component_values(source_component_value_emitter, line_number);
    }

    callback(
        CssStyleValueKind::GridPlacementShorthand,
        property_id,
        primitive_kind,
        line_number.is_some() && primitive_kind != CssPrimitiveValueKind::Invalid,
        numeric_value,
        false,
        0.0,
        (shorthand_property_id & 0xff) as u8,
        (shorthand_property_id >> 8) as u8,
        kind,
        0,
        source_or_unit.as_bytes(),
        name.unwrap_or(""),
    );
}

const CURSOR_CALLBACK_IMAGE: u8 = 0;
const CURSOR_CALLBACK_PREDEFINED: u8 = 1;
const CURSOR_CALLBACK_IMAGE_COORDINATE_X: u8 = 2;
const CURSOR_CALLBACK_IMAGE_COORDINATE_Y: u8 = 3;
const CONTENT_CALLBACK_NORMAL: u8 = 0;
const CONTENT_CALLBACK_NONE: u8 = 1;
const CONTENT_CALLBACK_ITEM_QUOTE: u8 = 2;
const CONTENT_CALLBACK_ITEM_STRING: u8 = 3;
const CONTENT_CALLBACK_ITEM_IMAGE: u8 = 4;
const CONTENT_CALLBACK_ITEM_COUNTER: u8 = 5;
const CONTENT_CALLBACK_ALT_TEXT_STRING: u8 = 6;
const CONTENT_CALLBACK_ALT_TEXT_COUNTER: u8 = 7;
const CONTENT_CALLBACK_COUNTER_JOIN_STRING: u8 = 8;
const CONTENT_CALLBACK_COUNTER_STYLE_NAME: u8 = 9;
const CONTENT_CALLBACK_COUNTER_STYLE_SYMBOLS: u8 = 10;
const CONTENT_CALLBACK_COUNTER_STYLE_SYMBOL: u8 = 11;
const COUNTER_CALLBACK_FUNCTION: u8 = 0;
const COUNTER_CALLBACK_JOIN_STRING: u8 = 1;
const COUNTER_CALLBACK_STYLE_NAME: u8 = 2;
const COUNTER_CALLBACK_STYLE_SYMBOLS: u8 = 3;
const COUNTER_CALLBACK_STYLE_SYMBOL: u8 = 4;
const COUNTER_FUNCTION_COUNTER: u8 = 0;
const COUNTER_FUNCTION_COUNTERS: u8 = 1;
const FILTER_VALUE_LIST_CALLBACK_NONE: u8 = 0;
const FILTER_VALUE_LIST_CALLBACK_URL: u8 = 1;
const FILTER_VALUE_LIST_CALLBACK_BLUR: u8 = 2;
const FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW: u8 = 3;
const FILTER_VALUE_LIST_CALLBACK_HUE_ROTATE: u8 = 4;
const FILTER_VALUE_LIST_CALLBACK_SIMPLE: u8 = 5;
const FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW_RADIUS: u8 = 6;
const FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW_COLOR: u8 = 7;
const FLEX_SHORTHAND_CALLBACK_NONE: u8 = 0;
const FLEX_SHORTHAND_CALLBACK_GROW: u8 = 1;
const FLEX_SHORTHAND_CALLBACK_SHRINK: u8 = 2;
const FLEX_SHORTHAND_CALLBACK_BASIS: u8 = 3;
const LIST_STYLE_IMAGE_CALLBACK_NONE: u8 = 0;
const LIST_STYLE_IMAGE_CALLBACK_SOURCE: u8 = 1;
const LIST_STYLE_TYPE_CALLBACK_NONE: u8 = 0;
const LIST_STYLE_TYPE_CALLBACK_STRING: u8 = 1;
const LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_NAME: u8 = 2;
const LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_SYMBOLS: u8 = 3;
const LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_SYMBOL: u8 = 4;

const SIMPLE_FILTER_FUNCTION_BRIGHTNESS: u8 = 0;
const SIMPLE_FILTER_FUNCTION_CONTRAST: u8 = 1;
const SIMPLE_FILTER_FUNCTION_GRAYSCALE: u8 = 2;
const SIMPLE_FILTER_FUNCTION_INVERT: u8 = 3;
const SIMPLE_FILTER_FUNCTION_OPACITY: u8 = 4;
const SIMPLE_FILTER_FUNCTION_SATURATE: u8 = 5;
const SIMPLE_FILTER_FUNCTION_SEPIA: u8 = 6;
const TRANSFORM_LONGHAND_CALLBACK_NONE: u8 = 0;
const TRANSFORM_LONGHAND_CALLBACK_FUNCTION: u8 = 1;
const TRANSFORM_LONGHAND_FUNCTION_ROTATE: u8 = 0;
const TRANSFORM_LONGHAND_FUNCTION_ROTATE_X: u8 = 1;
const TRANSFORM_LONGHAND_FUNCTION_ROTATE_Y: u8 = 2;
const TRANSFORM_LONGHAND_FUNCTION_ROTATE_Z: u8 = 3;
const TRANSFORM_LONGHAND_FUNCTION_ROTATE_3D: u8 = 4;
const TRANSFORM_LONGHAND_FUNCTION_TRANSLATE: u8 = 5;
const TRANSFORM_LONGHAND_FUNCTION_TRANSLATE_3D: u8 = 6;
const TRANSFORM_LONGHAND_FUNCTION_SCALE: u8 = 7;
const TRANSFORM_LONGHAND_FUNCTION_SCALE_3D: u8 = 8;
const TRANSFORMATION_CALLBACK_BEGIN_FUNCTION: u8 = 0;
const TRANSFORMATION_CALLBACK_ARGUMENT: u8 = 1;
const FONT_VARIANT_CALLBACK_NORMAL: u8 = 0;
const FONT_VARIANT_CALLBACK_SIMPLE: u8 = 1;
const FONT_VARIANT_CALLBACK_ALTERNATES_VALUE: u8 = 2;
const FONT_VARIANT_CALLBACK_ALTERNATES_FEATURE_VALUE_NAME: u8 = 3;
const FONT_VARIANT_CALLBACK_EAST_ASIAN_VALUE: u8 = 4;
const FONT_VARIANT_CALLBACK_NUMERIC_VALUE: u8 = 5;
const FONT_VARIANT_CALLBACK_LIGATURES_VALUE: u8 = 6;
const SHAPE_OUTSIDE_CALLBACK_NONE: u8 = 0;
const SHAPE_OUTSIDE_CALLBACK_IMAGE: u8 = 1;
const SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE: u8 = 2;
const SHAPE_OUTSIDE_CALLBACK_SHAPE_BOX: u8 = 3;
const IMAGE_URL_FUNCTION_TYPE_NONE: u8 = 0;
const IMAGE_URL_FUNCTION_TYPE_URL: u8 = 1;
const IMAGE_URL_FUNCTION_TYPE_SRC: u8 = 2;

fn image_url_function_type_callback_payload(function_type: CssUrlFunctionType) -> u8 {
    match function_type {
        CssUrlFunctionType::Url => IMAGE_URL_FUNCTION_TYPE_URL,
        CssUrlFunctionType::Src => IMAGE_URL_FUNCTION_TYPE_SRC,
    }
}

fn image_callback_payload(image: &RustOwnedImage) -> (u8, u8, &str) {
    if image.kind == RustOwnedImageKind::Url
        && let Some(url) = image.url.as_ref()
    {
        return (
            image.kind as u8,
            image_url_function_type_callback_payload(url.function_type),
            &url.url,
        );
    }
    (
        image.kind as u8,
        IMAGE_URL_FUNCTION_TYPE_NONE,
        image.source.as_deref().unwrap_or(""),
    )
}

fn url_callback_payload(url: &RustOwnedUrl) -> (u8, &str) {
    if let Some(url) = url.url.as_ref() {
        return (image_url_function_type_callback_payload(url.function_type), &url.url);
    }
    (IMAGE_URL_FUNCTION_TYPE_NONE, "")
}

fn callback_url_style_value<C>(callback: &mut C, property_id: u16, url: &RustOwnedUrl)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    let (url_function_type, payload) = url_callback_payload(url);
    callback(
        CssStyleValueKind::Url,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        0,
        0,
        0,
        url_function_type,
        payload.as_bytes(),
        property_value_type_name(PropertyValueType::Url),
    );
}

fn callback_paint_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    paint: &RustOwnedPaint,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    const PAINT_CALLBACK_NONE: f64 = 0.0;
    const PAINT_CALLBACK_COLOR: u8 = 1;
    const PAINT_CALLBACK_URL: f64 = 2.0;
    const PAINT_CALLBACK_FALLBACK_COLOR: u8 = 4;

    match paint {
        RustOwnedPaint::None => callback(
            CssStyleValueKind::Paint,
            property_id,
            CssPrimitiveValueKind::Invalid,
            true,
            PAINT_CALLBACK_NONE,
            false,
            0.0,
            0,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedPaint::Color(color) => {
            callback_rust_owned_color(
                callback,
                source_component_value_emitter,
                CssStyleValueKind::Paint,
                property_id,
                PAINT_CALLBACK_COLOR,
                color,
            );
        }
        RustOwnedPaint::Url { url, fallback_color } => {
            let (url_function_type, payload) = url_callback_payload(url);
            callback(
                CssStyleValueKind::Paint,
                property_id,
                CssPrimitiveValueKind::Invalid,
                true,
                PAINT_CALLBACK_URL,
                false,
                0.0,
                0,
                0,
                0,
                url_function_type,
                payload.as_bytes(),
                "",
            );
            if let Some(fallback_color) = fallback_color {
                callback_rust_owned_color(
                    callback,
                    source_component_value_emitter,
                    CssStyleValueKind::Paint,
                    property_id,
                    PAINT_CALLBACK_FALLBACK_COLOR,
                    fallback_color,
                );
            }
        }
    }
}

fn callback_corner_shape_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    corner_shape: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    if let RustOwnedNestedPrimitiveValue::Keyword(keyword) = corner_shape {
        callback(
            CssStyleValueKind::CornerShape,
            property_id,
            CssPrimitiveValueKind::Keyword,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            keyword.as_bytes(),
            property_value_type_name(PropertyValueType::CornerShape),
        );
    } else {
        callback_nested_primitive_with_source_component_values_and_calculation(
            callback,
            calculation_callback,
            source_component_value_emitter,
            CssStyleValueKind::CornerShape,
            property_id,
            1,
            0,
            corner_shape,
        );
    }
}

fn callback_image_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    image: &RustOwnedImage,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (image_kind, url_function_type, payload) = image_callback_payload(image);
    callback(
        CssStyleValueKind::Image,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        image_kind,
        0,
        0,
        url_function_type,
        payload.as_bytes(),
        "",
    );
    if !image.component_values.is_empty() {
        source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.component_values);
    }
}

fn callback_image_set_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    image_set: &RustOwnedImageSet,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for option in &image_set.options {
        let (_, url_function_type, image_source) = image_callback_payload(&option.image);
        let metadata = image_set_option_metadata(option);
        callback(
            CssStyleValueKind::Image,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            RustOwnedImageKind::ImageSet as u8,
            option.image.kind as u8,
            0,
            url_function_type,
            image_source.as_bytes(),
            &metadata,
        );
        if !option.image.component_values.is_empty() {
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &option.image.component_values);
        }
        if !option.resolution_component_values.is_empty() {
            source_component_value_emitter.emit(
                SOURCE_COMPONENT_VALUE_LIST_IMAGE_SET_RESOLUTION,
                &option.resolution_component_values,
            );
        }
    }
}

fn image_set_option_metadata(option: &RustOwnedImageSetOption) -> String {
    format!(
        "{}\0{}",
        option.resolution.as_deref().unwrap_or(""),
        option.mime_type.as_deref().unwrap_or("")
    )
}

fn callback_cursor_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedCursor,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for image in &value.images {
        let (image_kind, url_function_type, payload) = image_callback_payload(&image.image);
        callback(
            CssStyleValueKind::Cursor,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            CURSOR_CALLBACK_IMAGE,
            image_kind,
            0,
            url_function_type,
            payload.as_bytes(),
            "",
        );
        if !image.image.component_values.is_empty() {
            source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.image.component_values);
        }
        if let (Some(x), Some(y)) = (&image.x, &image.y) {
            callback_nested_primitive_with_source_component_values(
                callback,
                source_component_value_emitter,
                CssStyleValueKind::Cursor,
                property_id,
                CURSOR_CALLBACK_IMAGE_COORDINATE_X,
                0,
                x,
            );
            callback_nested_primitive_with_source_component_values(
                callback,
                source_component_value_emitter,
                CssStyleValueKind::Cursor,
                property_id,
                CURSOR_CALLBACK_IMAGE_COORDINATE_Y,
                0,
                y,
            );
        }
    }

    callback(
        CssStyleValueKind::Cursor,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        CURSOR_CALLBACK_PREDEFINED,
        0,
        0,
        0,
        value.predefined.as_bytes(),
        "",
    );
}

fn callback_list_style_image<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedListStyleImage,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (kind, image_kind, url_function_type, payload) = match value {
        RustOwnedListStyleImage::None => (LIST_STYLE_IMAGE_CALLBACK_NONE, 0, IMAGE_URL_FUNCTION_TYPE_NONE, ""),
        RustOwnedListStyleImage::Image(image) => {
            let (image_kind, url_function_type, payload) = image_callback_payload(image);
            (LIST_STYLE_IMAGE_CALLBACK_SOURCE, image_kind, url_function_type, payload)
        }
    };
    callback(
        CssStyleValueKind::ListStyle,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        1,
        kind,
        image_kind,
        url_function_type,
        payload.as_bytes(),
        "",
    );
    if let RustOwnedListStyleImage::Image(image) = value
        && !image.component_values.is_empty()
    {
        source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.component_values);
    }
}

fn callback_list_style_type<C>(callback: &mut C, property_id: u16, value: &RustOwnedListStyleType)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedListStyleType::None => {
            callback_list_style_type_event(callback, property_id, LIST_STYLE_TYPE_CALLBACK_NONE, 0, "");
        }
        RustOwnedListStyleType::String(source) => {
            callback_list_style_type_event(callback, property_id, LIST_STYLE_TYPE_CALLBACK_STRING, 0, source);
        }
        RustOwnedListStyleType::CounterStyle(CounterStyle::Name(name)) => {
            callback_list_style_type_event(
                callback,
                property_id,
                LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_NAME,
                0,
                name,
            );
        }
        RustOwnedListStyleType::CounterStyle(CounterStyle::SymbolsFunction { symbols_type, symbols }) => {
            callback_list_style_type_event(
                callback,
                property_id,
                LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_SYMBOLS,
                *symbols_type as u8,
                "",
            );
            for symbol in symbols {
                callback_list_style_type_event(
                    callback,
                    property_id,
                    LIST_STYLE_TYPE_CALLBACK_COUNTER_STYLE_SYMBOL,
                    0,
                    symbol,
                );
            }
        }
    }
}

fn callback_list_style_type_event<C>(callback: &mut C, property_id: u16, kind: u8, symbols_type: u8, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::ListStyle,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        2,
        kind,
        symbols_type,
        0,
        source.as_bytes(),
        "",
    );
}

fn callback_content_style_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedContent,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedContent::Normal => callback_content_event(callback, property_id, CONTENT_CALLBACK_NORMAL, ""),
        RustOwnedContent::None => callback_content_event(callback, property_id, CONTENT_CALLBACK_NONE, ""),
        RustOwnedContent::Items { items, alt_text } => {
            for item in items {
                match item {
                    RustOwnedContentItem::Quote(source) => {
                        callback_content_event(callback, property_id, CONTENT_CALLBACK_ITEM_QUOTE, source);
                    }
                    RustOwnedContentItem::String(source) => {
                        callback_content_event(callback, property_id, CONTENT_CALLBACK_ITEM_STRING, source);
                    }
                    RustOwnedContentItem::Image(image) => {
                        callback_content_image_event(callback, source_component_value_emitter, property_id, image);
                    }
                    RustOwnedContentItem::Counter(counter) => {
                        callback_content_counter_event(callback, property_id, CONTENT_CALLBACK_ITEM_COUNTER, counter);
                    }
                }
            }
            for item in alt_text {
                match item {
                    RustOwnedContentAltTextItem::String(source) => {
                        callback_content_event(callback, property_id, CONTENT_CALLBACK_ALT_TEXT_STRING, source);
                    }
                    RustOwnedContentAltTextItem::Counter(counter) => {
                        callback_content_counter_event(
                            callback,
                            property_id,
                            CONTENT_CALLBACK_ALT_TEXT_COUNTER,
                            counter,
                        );
                    }
                }
            }
        }
    }
}

fn callback_content_counter_event<C>(callback: &mut C, property_id: u16, kind: u8, counter: &RustOwnedCounterFunction)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::Content,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        match counter.function {
            RustOwnedCounterFunctionKind::Counter => COUNTER_FUNCTION_COUNTER,
            RustOwnedCounterFunctionKind::Counters => COUNTER_FUNCTION_COUNTERS,
        },
        0,
        0,
        counter.counter_name.as_bytes(),
        "",
    );

    if let Some(join_string) = counter.join_string.as_ref() {
        callback_content_event(callback, property_id, CONTENT_CALLBACK_COUNTER_JOIN_STRING, join_string);
    }

    if let Some(counter_style) = counter.counter_style.as_ref() {
        match counter_style {
            CounterStyle::Name(name) => {
                callback_content_event(callback, property_id, CONTENT_CALLBACK_COUNTER_STYLE_NAME, name);
            }
            CounterStyle::SymbolsFunction { symbols_type, symbols } => {
                callback(
                    CssStyleValueKind::Content,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    CONTENT_CALLBACK_COUNTER_STYLE_SYMBOLS,
                    *symbols_type as u8,
                    0,
                    0,
                    &[],
                    "",
                );
                for symbol in symbols {
                    callback_content_event(callback, property_id, CONTENT_CALLBACK_COUNTER_STYLE_SYMBOL, symbol);
                }
            }
        }
    }
}

fn callback_content_image_event<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    image: &RustOwnedImage,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (image_kind, url_function_type, payload) = image_callback_payload(image);
    callback(
        CssStyleValueKind::Content,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        CONTENT_CALLBACK_ITEM_IMAGE,
        image_kind,
        0,
        url_function_type,
        payload.as_bytes(),
        "",
    );
    if !image.component_values.is_empty() {
        source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.component_values);
    }
}

fn callback_counter_function_style_value<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    counter: &RustOwnedCounterFunction,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        style_value_kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        COUNTER_CALLBACK_FUNCTION,
        match counter.function {
            RustOwnedCounterFunctionKind::Counter => COUNTER_FUNCTION_COUNTER,
            RustOwnedCounterFunctionKind::Counters => COUNTER_FUNCTION_COUNTERS,
        },
        0,
        0,
        counter.counter_name.as_bytes(),
        "",
    );

    if let Some(join_string) = counter.join_string.as_ref() {
        callback(
            style_value_kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            COUNTER_CALLBACK_JOIN_STRING,
            0,
            0,
            0,
            join_string.as_bytes(),
            "",
        );
    }

    if let Some(counter_style) = counter.counter_style.as_ref() {
        match counter_style {
            CounterStyle::Name(name) => {
                callback(
                    style_value_kind,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    COUNTER_CALLBACK_STYLE_NAME,
                    0,
                    0,
                    0,
                    name.as_bytes(),
                    "",
                );
            }
            CounterStyle::SymbolsFunction { symbols_type, symbols } => {
                callback(
                    style_value_kind,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    COUNTER_CALLBACK_STYLE_SYMBOLS,
                    *symbols_type as u8,
                    0,
                    0,
                    &[],
                    "",
                );
                for symbol in symbols {
                    callback(
                        style_value_kind,
                        property_id,
                        CssPrimitiveValueKind::Invalid,
                        false,
                        0.0,
                        false,
                        0.0,
                        COUNTER_CALLBACK_STYLE_SYMBOL,
                        0,
                        0,
                        0,
                        symbol.as_bytes(),
                        "",
                    );
                }
            }
        }
    }
}

fn callback_content_event<C>(callback: &mut C, property_id: u16, kind: u8, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::Content,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        0,
        0,
        0,
        source.as_bytes(),
        "",
    );
}

fn callback_counter_style<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    counter_style: &CounterStyle,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match counter_style {
        CounterStyle::Name(name) => {
            callback(
                style_value_kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                COUNTER_CALLBACK_STYLE_NAME,
                0,
                0,
                0,
                name.as_bytes(),
                "",
            );
        }
        CounterStyle::SymbolsFunction { symbols_type, symbols } => {
            callback(
                style_value_kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                COUNTER_CALLBACK_STYLE_SYMBOLS,
                *symbols_type as u8,
                0,
                0,
                &[],
                "",
            );
            for symbol in symbols {
                callback(
                    style_value_kind,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    COUNTER_CALLBACK_STYLE_SYMBOL,
                    0,
                    0,
                    0,
                    symbol.as_bytes(),
                    "",
                );
            }
        }
    }
}

fn callback_flex_basis<C, S, E>(
    callback: &mut C,
    filtered_input: &str,
    source_component_value_list_callback: &mut S,
    source_component_value_callback: &mut E,
    property_id: u16,
    value: &RustOwnedFlexBasis,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedFlexBasis::Value(value) => match value {
            RustOwnedNestedPrimitiveValue::Keyword(keyword) => {
                let Some(kind) = flex_basis_kind_from_keyword(keyword) else {
                    unreachable!("flex-basis keywords are validated while parsing")
                };
                callback(
                    CssStyleValueKind::Flex,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    FLEX_SHORTHAND_CALLBACK_BASIS,
                    kind,
                    0,
                    0,
                    &[],
                    "",
                );
            }
            RustOwnedNestedPrimitiveValue::Source {
                source,
                component_values,
            } => {
                callback(
                    CssStyleValueKind::Flex,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    FLEX_SHORTHAND_CALLBACK_BASIS,
                    FLEX_BASIS_KIND_SOURCE,
                    0,
                    0,
                    source.as_bytes(),
                    "",
                );
                source_component_value_list_callback(SOURCE_COMPONENT_VALUE_LIST_FLEX_BASIS);
                emit_component_values(component_values, filtered_input, source_component_value_callback);
            }
            _ => callback_nested_primitive_with_source_component_values(
                callback,
                &mut SourceComponentValueEmitter {
                    filtered_input,
                    list_callback: source_component_value_list_callback,
                    component_value_callback: source_component_value_callback,
                },
                CssStyleValueKind::Flex,
                property_id,
                FLEX_SHORTHAND_CALLBACK_BASIS,
                FLEX_BASIS_KIND_LENGTH_PERCENTAGE,
                value,
            ),
        },
        RustOwnedFlexBasis::FitContentFunction(value) => callback_nested_primitive_with_source_component_values(
            callback,
            &mut SourceComponentValueEmitter {
                filtered_input,
                list_callback: source_component_value_list_callback,
                component_value_callback: source_component_value_callback,
            },
            CssStyleValueKind::Flex,
            property_id,
            FLEX_SHORTHAND_CALLBACK_BASIS,
            FLEX_BASIS_KIND_FIT_CONTENT_FUNCTION,
            value,
        ),
    }
}

fn flex_basis_kind_from_keyword(keyword: &str) -> Option<u8> {
    if keyword == "auto" {
        return Some(FLEX_BASIS_KIND_AUTO);
    }
    if keyword == "content" {
        return Some(FLEX_BASIS_KIND_CONTENT);
    }
    if keyword == "fit-content" {
        return Some(FLEX_BASIS_KIND_FIT_CONTENT);
    }
    if keyword == "min-content" {
        return Some(FLEX_BASIS_KIND_MIN_CONTENT);
    }
    if keyword == "max-content" {
        return Some(FLEX_BASIS_KIND_MAX_CONTENT);
    }
    None
}

fn callback_shape_outside_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedShapeOutside,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedShapeOutside::None => {
            callback_shape_outside_event(callback, property_id, SHAPE_OUTSIDE_CALLBACK_NONE, 0, "");
        }
        RustOwnedShapeOutside::Image(image) => {
            callback_shape_outside_image_event(callback, source_component_value_emitter, property_id, image);
        }
        RustOwnedShapeOutside::Shape { basic_shape, shape_box } => {
            if let Some(basic_shape) = basic_shape {
                callback_shape_outside_basic_shape_event(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    property_id,
                    basic_shape,
                );
            }
            if let Some(shape_box) = shape_box {
                callback(
                    CssStyleValueKind::ShapeOutside,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    SHAPE_OUTSIDE_CALLBACK_SHAPE_BOX,
                    *shape_box as u8,
                    0,
                    0,
                    &[],
                    "",
                );
            }
        }
    }
}

fn callback_shape_outside_event<C>(callback: &mut C, property_id: u16, kind: u8, image_kind: u8, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        image_kind,
        0,
        0,
        source.as_bytes(),
        "",
    );
}

fn callback_shape_outside_image_event<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    image: &RustOwnedImage,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (image_kind, url_function_type, payload) = image_callback_payload(image);
    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        SHAPE_OUTSIDE_CALLBACK_IMAGE,
        image_kind,
        0,
        url_function_type,
        payload.as_bytes(),
        "",
    );
    if !image.component_values.is_empty() {
        source_component_value_emitter.emit(SOURCE_COMPONENT_VALUE_LIST_IMAGE, &image.component_values);
    }
}

fn callback_shape_outside_basic_shape_event<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (kind, path_data) = basic_shape_callback_payload(value);

    if matches!(
        value.kind,
        RustOwnedBasicShapeKind::Inset | RustOwnedBasicShapeKind::Xywh | RustOwnedBasicShapeKind::Rect
    ) {
        callback_shape_outside_basic_shape_header(callback, property_id, kind, value.fill_rule);
        callback_shape_outside_basic_shape_rectangle_components(
            callback,
            calculation_callback,
            source_component_value_emitter,
            property_id,
            kind,
            value,
        );
        return;
    }

    if matches!(
        value.kind,
        RustOwnedBasicShapeKind::Circle | RustOwnedBasicShapeKind::Ellipse
    ) {
        callback_shape_outside_basic_shape_header(callback, property_id, kind, value.fill_rule);
        callback_shape_outside_basic_shape_radial_components(
            callback,
            calculation_callback,
            source_component_value_emitter,
            property_id,
            kind,
            value,
        );
        return;
    }

    if value.kind == RustOwnedBasicShapeKind::Polygon {
        callback_shape_outside_basic_shape_header(callback, property_id, kind, value.fill_rule);
        for point in &value.polygon_points {
            callback_shape_outside_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                kind,
                value.fill_rule,
                BASIC_SHAPE_COMPONENT_POLYGON_POINT_X,
                &point.x,
            );
            callback_shape_outside_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                kind,
                value.fill_rule,
                BASIC_SHAPE_COMPONENT_POLYGON_POINT_Y,
                &point.y,
            );
        }
        return;
    }

    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
        kind,
        value.fill_rule as u8,
        0,
        path_data.as_bytes(),
        "",
    );
}

fn callback_shape_outside_basic_shape_header<C>(
    callback: &mut C,
    property_id: u16,
    kind: u8,
    fill_rule: RustOwnedBasicShapeFillRule,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
        kind,
        BASIC_SHAPE_COMPONENT_HEADER,
        fill_rule as u8,
        &[],
        "",
    );
}

fn callback_shape_outside_basic_shape_rectangle_components<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for component in &value.rectangle_components {
        match component {
            RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => callback(
                CssStyleValueKind::ShapeOutside,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
                kind,
                BASIC_SHAPE_COMPONENT_RECTANGLE_AUTO,
                0,
                &[],
                "",
            ),
            _ => {
                callback_shape_outside_basic_shape_nested_primitive(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    property_id,
                    kind,
                    RustOwnedBasicShapeFillRule::Nonzero,
                    BASIC_SHAPE_COMPONENT_RECTANGLE_LENGTH_PERCENTAGE,
                    component,
                );
            }
        }
    }

    if let Some(border_radius) = &value.rectangle_border_radius {
        for radius in &border_radius.horizontal_radii {
            callback_shape_outside_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                kind,
                RustOwnedBasicShapeFillRule::Nonzero,
                BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_HORIZONTAL,
                radius,
            );
        }
        for radius in &border_radius.vertical_radii {
            callback_shape_outside_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                kind,
                RustOwnedBasicShapeFillRule::Nonzero,
                BASIC_SHAPE_COMPONENT_RECTANGLE_BORDER_RADIUS_VERTICAL,
                radius,
            );
        }
    }
}

fn callback_shape_outside_basic_shape_radial_components<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: &RustOwnedBasicShape,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    for component in &value.radial_shape_radius {
        if let RustOwnedNestedPrimitiveValue::Keyword(keyword) = component {
            let Some(extent) = radial_extent_from_keyword(keyword) else {
                unreachable!("radial shape radius keywords are validated while parsing")
            };
            callback(
                CssStyleValueKind::ShapeOutside,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
                kind,
                BASIC_SHAPE_COMPONENT_RADIAL_EXTENT,
                extent as u8,
                &[],
                "",
            );
        } else {
            callback_shape_outside_basic_shape_nested_primitive(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                kind,
                RustOwnedBasicShapeFillRule::Nonzero,
                BASIC_SHAPE_COMPONENT_RADIAL_LENGTH_PERCENTAGE,
                component,
            );
        }
    }

    if let Some(position) = &value.radial_shape_position {
        callback_shape_outside_basic_shape_position_component(
            callback,
            calculation_callback,
            source_component_value_emitter,
            property_id,
            kind,
            BASIC_SHAPE_COMPONENT_RADIAL_POSITION_X,
            &position.x,
        );
        callback_shape_outside_basic_shape_position_component(
            callback,
            calculation_callback,
            source_component_value_emitter,
            property_id,
            kind,
            BASIC_SHAPE_COMPONENT_RADIAL_POSITION_Y,
            &position.y,
        );
    }
}

fn callback_shape_outside_basic_shape_position_component<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    component_kind: u8,
    component: &RustOwnedPositionComponent,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let edge = component.edge.map_or(0, rust_owned_position_edge_to_callback_value);
    let Some(offset) = component.offset.as_ref() else {
        callback(
            CssStyleValueKind::ShapeOutside,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
            kind,
            component_kind,
            edge,
            &[],
            "",
        );
        return;
    };

    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(offset);
    emit_nested_primitive_source_component_values(source_component_value_emitter, offset);
    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(offset),
        numeric_value,
        false,
        0.0,
        SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
        kind,
        component_kind,
        edge | 0x80,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = offset {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

#[allow(clippy::too_many_arguments)]
fn callback_shape_outside_basic_shape_nested_primitive<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    fill_rule: RustOwnedBasicShapeFillRule,
    component: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);

    emit_nested_primitive_source_component_values(source_component_value_emitter, value);
    callback(
        CssStyleValueKind::ShapeOutside,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(value),
        numeric_value,
        false,
        0.0,
        SHAPE_OUTSIDE_CALLBACK_BASIC_SHAPE,
        kind,
        component,
        fill_rule as u8,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = value {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn callback_filter_value_list_style_value<C, D, U, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    url_modifier_callback: &mut U,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedFilterValueList,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    U: FnMut(&UrlModifier),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedFilterValueList::None => callback(
            CssStyleValueKind::FilterValueList,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            FILTER_VALUE_LIST_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedFilterValueList::Filters(filters) => {
            for filter in filters {
                match filter {
                    RustOwnedFilterValue::Url(url) => {
                        let (url_function_type, payload) = url_callback_payload(url);
                        callback(
                            CssStyleValueKind::FilterValueList,
                            property_id,
                            CssPrimitiveValueKind::Invalid,
                            false,
                            0.0,
                            false,
                            0.0,
                            FILTER_VALUE_LIST_CALLBACK_URL,
                            0,
                            0,
                            url_function_type,
                            payload.as_bytes(),
                            "",
                        );
                        if let Some(url) = &url.url {
                            for modifier in &url.request_url_modifiers {
                                url_modifier_callback(modifier);
                            }
                        }
                    }
                    RustOwnedFilterValue::Blur { radius } => callback_optional_filter_nested_primitive(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        property_id,
                        FILTER_VALUE_LIST_CALLBACK_BLUR,
                        0,
                        radius.as_ref(),
                    ),
                    RustOwnedFilterValue::DropShadow {
                        color,
                        offset_x,
                        offset_y,
                        radius,
                    } => {
                        callback_nested_primitive_pair_with_source_component_values(
                            callback,
                            source_component_value_emitter,
                            CssStyleValueKind::FilterValueList,
                            property_id,
                            FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW,
                            0,
                            offset_x,
                            offset_y,
                        );
                        if let Some(radius) = radius {
                            callback_filter_nested_primitive(
                                callback,
                                calculation_callback,
                                source_component_value_emitter,
                                property_id,
                                FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW_RADIUS,
                                0,
                                radius,
                            );
                        }
                        if let Some(color) = color {
                            callback_rust_owned_color(
                                callback,
                                source_component_value_emitter,
                                CssStyleValueKind::FilterValueList,
                                property_id,
                                FILTER_VALUE_LIST_CALLBACK_DROP_SHADOW_COLOR,
                                color,
                            );
                        }
                    }
                    RustOwnedFilterValue::HueRotate { angle } => callback_optional_filter_nested_primitive(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        property_id,
                        FILTER_VALUE_LIST_CALLBACK_HUE_ROTATE,
                        0,
                        angle.as_ref(),
                    ),
                    RustOwnedFilterValue::Simple { function, amount } => callback_optional_filter_nested_primitive(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        property_id,
                        FILTER_VALUE_LIST_CALLBACK_SIMPLE,
                        match function {
                            RustOwnedSimpleFilterFunction::Brightness => SIMPLE_FILTER_FUNCTION_BRIGHTNESS,
                            RustOwnedSimpleFilterFunction::Contrast => SIMPLE_FILTER_FUNCTION_CONTRAST,
                            RustOwnedSimpleFilterFunction::Grayscale => SIMPLE_FILTER_FUNCTION_GRAYSCALE,
                            RustOwnedSimpleFilterFunction::Invert => SIMPLE_FILTER_FUNCTION_INVERT,
                            RustOwnedSimpleFilterFunction::Opacity => SIMPLE_FILTER_FUNCTION_OPACITY,
                            RustOwnedSimpleFilterFunction::Saturate => SIMPLE_FILTER_FUNCTION_SATURATE,
                            RustOwnedSimpleFilterFunction::Sepia => SIMPLE_FILTER_FUNCTION_SEPIA,
                        },
                        amount.as_ref(),
                    ),
                }
            }
        }
    }
}

fn callback_filter_source<C>(callback: &mut C, property_id: u16, kind: u8, secondary_kind: u8, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::FilterValueList,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        kind,
        secondary_kind,
        1,
        0,
        source.as_bytes(),
        "",
    );
}

fn callback_filter_nested_primitive<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    callback_nested_primitive_with_source_component_values_and_calculation(
        callback,
        calculation_callback,
        source_component_value_emitter,
        CssStyleValueKind::FilterValueList,
        property_id,
        kind,
        secondary_kind,
        value,
    );
}

fn callback_optional_filter_nested_primitive<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: Option<&RustOwnedNestedPrimitiveValue>,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    if let Some(value) = value {
        callback_filter_nested_primitive(
            callback,
            calculation_callback,
            source_component_value_emitter,
            property_id,
            kind,
            secondary_kind,
            value,
        );
    } else {
        callback(
            CssStyleValueKind::FilterValueList,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            kind,
            secondary_kind,
            0,
            0,
            &[],
            "",
        );
    }
}

fn callback_optional_column_integer<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: Option<&RustOwnedNestedPrimitiveValue>,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        Some(RustOwnedNestedPrimitiveValue::Keyword(keyword)) if keyword == "auto" => callback(
            CssStyleValueKind::Columns,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            kind,
            1,
            0,
            0,
            &[],
            "",
        ),
        Some(value) => callback_nested_primitive_with_source_component_values(
            callback,
            source_component_value_emitter,
            CssStyleValueKind::Columns,
            property_id,
            kind,
            0,
            value,
        ),
        None => {}
    }
}

fn callback_optional_column_length<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: Option<&RustOwnedNestedPrimitiveValue>,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        Some(RustOwnedNestedPrimitiveValue::Keyword(keyword)) if keyword == "auto" => callback(
            CssStyleValueKind::Columns,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            kind,
            1,
            0,
            0,
            &[],
            "",
        ),
        Some(value) => callback_nested_primitive_with_source_component_values(
            callback,
            source_component_value_emitter,
            CssStyleValueKind::Columns,
            property_id,
            kind,
            0,
            value,
        ),
        None => {}
    }
}

fn callback_nested_primitive_pair<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
    secondary_value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);
    let (secondary_primitive_kind, secondary_numeric_value, secondary_unit_or_source) =
        nested_primitive_callback_payload(secondary_value);

    callback(
        style_value_kind,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(value),
        numeric_value,
        nested_primitive_callback_has_numeric_value(secondary_value),
        secondary_numeric_value,
        kind,
        secondary_kind,
        1,
        secondary_primitive_kind as u8,
        unit_or_source.as_bytes(),
        secondary_unit_or_source,
    );
}

#[allow(clippy::too_many_arguments)]
fn callback_nested_primitive_pair_with_source_component_values<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
    secondary_value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    emit_nested_primitive_source_component_values(source_component_value_emitter, value);
    emit_secondary_nested_primitive_source_component_values(source_component_value_emitter, secondary_value);
    callback_nested_primitive_pair(
        callback,
        style_value_kind,
        property_id,
        kind,
        secondary_kind,
        value,
        secondary_value,
    );
}

fn callback_nested_primitive<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(value);

    callback(
        style_value_kind,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(value),
        numeric_value,
        false,
        0.0,
        kind,
        secondary_kind,
        1,
        0,
        unit_or_source.as_bytes(),
        "",
    );
}

fn callback_nested_primitive_with_source_component_values<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    emit_nested_primitive_source_component_values(source_component_value_emitter, value);
    callback_nested_primitive(callback, style_value_kind, property_id, kind, secondary_kind, value);
}

#[allow(clippy::too_many_arguments)]
fn callback_nested_primitive_with_source_component_values_and_calculation<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    emit_nested_primitive_source_component_values(source_component_value_emitter, value);
    callback_nested_primitive_with_calculation(
        callback,
        calculation_callback,
        style_value_kind,
        property_id,
        kind,
        secondary_kind,
        value,
    );
}

fn callback_nested_primitive_with_calculation<C, D>(
    callback: &mut C,
    calculation_callback: &mut D,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    kind: u8,
    secondary_kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
{
    callback_nested_primitive(callback, style_value_kind, property_id, kind, secondary_kind, value);
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = value {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn callback_view_timeline_inset_value<C, S, E>(
    callback: &mut C,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedNestedPrimitiveValue::Keyword(keyword) if keyword == "auto" => {
            let auto_kind = if style_value_kind == CssStyleValueKind::ViewTimeline {
                2
            } else {
                0
            };
            callback(
                style_value_kind,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                auto_kind,
                0,
                0,
                0,
                &[],
                property_value_type_name(PropertyValueType::ViewTimelineInset),
            );
        }
        _ => {
            let length_percentage_kind = if style_value_kind == CssStyleValueKind::ViewTimeline {
                3
            } else {
                1
            };
            callback_nested_primitive_with_source_component_values(
                callback,
                source_component_value_emitter,
                style_value_kind,
                property_id,
                length_percentage_kind,
                0,
                value,
            );
        }
    }
}

fn callback_view_timeline_inset_count<C>(
    callback: &mut C,
    style_value_kind: CssStyleValueKind,
    property_id: u16,
    count: usize,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    let count_kind = if style_value_kind == CssStyleValueKind::ViewTimeline {
        1
    } else {
        2
    };
    callback(
        style_value_kind,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        count_kind,
        count as u8,
        0,
        0,
        &[],
        "",
    );
}

fn callback_position_component<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    component_kind: u8,
    component: &RustOwnedPositionComponent,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let edge = component.edge.map_or(0, rust_owned_position_edge_to_callback_value);
    let Some(offset) = component.offset.as_ref() else {
        callback(
            CssStyleValueKind::Position,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            component_kind,
            edge,
            0,
            0,
            &[],
            "",
        );
        return;
    };

    emit_nested_primitive_source_component_values(source_component_value_emitter, offset);
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(offset);
    callback(
        CssStyleValueKind::Position,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(offset),
        numeric_value,
        false,
        0.0,
        component_kind,
        edge,
        1,
        0,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = offset {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn rust_owned_position_edge_to_callback_value(edge: PositionEdge) -> u8 {
    match edge {
        PositionEdge::Center => 1,
        PositionEdge::Left => 2,
        PositionEdge::Right => 3,
        PositionEdge::Top => 4,
        PositionEdge::Bottom => 5,
    }
}

fn callback_transform_origin_component<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    callback_nested_primitive_with_source_component_values_and_calculation(
        callback,
        calculation_callback,
        source_component_value_emitter,
        CssStyleValueKind::TransformOrigin,
        property_id,
        kind,
        0,
        value,
    );
}

fn callback_background_size<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedBackgroundSize,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    const KEYWORD: u8 = 0;
    const WIDTH: u8 = 1;
    const HEIGHT: u8 = 2;

    match value {
        RustOwnedBackgroundSize::Cover => callback_background_size_keyword(callback, property_id, KEYWORD, "cover"),
        RustOwnedBackgroundSize::Contain => callback_background_size_keyword(callback, property_id, KEYWORD, "contain"),
        RustOwnedBackgroundSize::Explicit { width, height } => {
            callback_background_size_component(
                callback,
                calculation_callback,
                source_component_value_emitter,
                property_id,
                WIDTH,
                width,
            );
            if let Some(height) = height {
                callback_background_size_component(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    property_id,
                    HEIGHT,
                    height,
                );
            }
        }
    }
}

fn callback_background_size_keyword<C>(callback: &mut C, property_id: u16, kind: u8, keyword: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::BackgroundSize,
        property_id,
        CssPrimitiveValueKind::Keyword,
        false,
        0.0,
        false,
        0.0,
        kind,
        0,
        1,
        0,
        keyword.as_bytes(),
        "",
    );
}

fn callback_background_size_component<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    kind: u8,
    value: &RustOwnedNestedPrimitiveValue,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    callback_nested_primitive_with_source_component_values_and_calculation(
        callback,
        calculation_callback,
        source_component_value_emitter,
        CssStyleValueKind::BackgroundSize,
        property_id,
        kind,
        0,
        value,
    );
}

fn nested_primitive_callback_payload(value: &RustOwnedNestedPrimitiveValue) -> (CssPrimitiveValueKind, f64, &str) {
    match value {
        RustOwnedNestedPrimitiveValue::Number(value) => (CssPrimitiveValueKind::Number, *value, ""),
        RustOwnedNestedPrimitiveValue::Percentage(value) => (CssPrimitiveValueKind::Percentage, *value, ""),
        RustOwnedNestedPrimitiveValue::Integer(value) => (CssPrimitiveValueKind::Integer, *value as f64, ""),
        RustOwnedNestedPrimitiveValue::Length { value, unit } => (CssPrimitiveValueKind::Length, *value, unit),
        RustOwnedNestedPrimitiveValue::Angle { value, unit } => (CssPrimitiveValueKind::Angle, *value, unit),
        RustOwnedNestedPrimitiveValue::Flex { value, unit } => (CssPrimitiveValueKind::Flex, *value, unit),
        RustOwnedNestedPrimitiveValue::Frequency { value, unit } => (CssPrimitiveValueKind::Frequency, *value, unit),
        RustOwnedNestedPrimitiveValue::Resolution { value, unit } => (CssPrimitiveValueKind::Resolution, *value, unit),
        RustOwnedNestedPrimitiveValue::Time { value, unit } => (CssPrimitiveValueKind::Time, *value, unit),
        RustOwnedNestedPrimitiveValue::Keyword(keyword) => (CssPrimitiveValueKind::Keyword, 0.0, keyword),
        RustOwnedNestedPrimitiveValue::MathFunction(value) => (CssPrimitiveValueKind::Invalid, 0.0, &value.source),
        RustOwnedNestedPrimitiveValue::TreeCountingFunction(value) => (
            CssPrimitiveValueKind::Invalid,
            0.0,
            match value.function {
                RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count()",
                RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index()",
            },
        ),
        RustOwnedNestedPrimitiveValue::Source { source, .. } => (CssPrimitiveValueKind::Invalid, 0.0, source),
    }
}

fn nested_primitive_callback_has_numeric_value(value: &RustOwnedNestedPrimitiveValue) -> bool {
    !matches!(
        value,
        RustOwnedNestedPrimitiveValue::Keyword(_)
            | RustOwnedNestedPrimitiveValue::MathFunction(_)
            | RustOwnedNestedPrimitiveValue::TreeCountingFunction(_)
            | RustOwnedNestedPrimitiveValue::Source { .. }
    )
}

fn callback_font_variant_longhand_style_value<C>(
    callback: &mut C,
    property_id: u16,
    value: &RustOwnedFontVariantLonghand,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedFontVariantLonghand::Alternates(values) => {
            for value in values {
                let feature_value_names = null_separated_string_list_bytes(&value.feature_value_names);
                callback(
                    CssStyleValueKind::FontVariantAlternates,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    value.kind as u8,
                    0,
                    0,
                    0,
                    &feature_value_names,
                    "",
                );
            }
        }
        RustOwnedFontVariantLonghand::EastAsian(values) => {
            for value in values {
                callback(
                    CssStyleValueKind::FontVariantEastAsian,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    value.kind as u8,
                    0,
                    0,
                    0,
                    value.value.as_bytes(),
                    "",
                );
            }
        }
        RustOwnedFontVariantLonghand::Ligatures(values) => {
            for value in values {
                callback(
                    CssStyleValueKind::FontVariantLigatures,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    value.kind as u8,
                    0,
                    0,
                    0,
                    value.value.as_bytes(),
                    "",
                );
            }
        }
        RustOwnedFontVariantLonghand::Numeric(values) => {
            for value in values {
                callback(
                    CssStyleValueKind::FontVariantNumeric,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    value.kind as u8,
                    0,
                    0,
                    0,
                    value.value.as_bytes(),
                    "",
                );
            }
        }
    }
}

fn callback_font_variant_style_value<C>(callback: &mut C, property_id: u16, value: &FontVariant)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    if !value.has_any_value() {
        callback(
            CssStyleValueKind::FontVariant,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            FONT_VARIANT_CALLBACK_NORMAL,
            0,
            0,
            0,
            &[],
            "",
        );
        return;
    }

    if value.ligatures_none {
        callback_font_variant_simple_value(
            callback,
            property_id,
            CssFontVariantSimpleValueKind::LigaturesNone,
            None,
        );
    }
    if let Some(alternates) = &value.alternates {
        for value in alternates {
            callback(
                CssStyleValueKind::FontVariant,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                FONT_VARIANT_CALLBACK_ALTERNATES_VALUE,
                value.kind as u8,
                0,
                0,
                &[],
                "",
            );
            for feature_value_name in &value.feature_value_names {
                callback(
                    CssStyleValueKind::FontVariant,
                    property_id,
                    CssPrimitiveValueKind::Invalid,
                    false,
                    0.0,
                    false,
                    0.0,
                    FONT_VARIANT_CALLBACK_ALTERNATES_FEATURE_VALUE_NAME,
                    0,
                    0,
                    0,
                    feature_value_name.as_bytes(),
                    "",
                );
            }
        }
    }
    if let Some(caps) = &value.caps {
        callback_font_variant_simple_value(callback, property_id, CssFontVariantSimpleValueKind::Caps, Some(caps));
    }
    if let Some(east_asian) = &value.east_asian {
        for value in east_asian {
            callback(
                CssStyleValueKind::FontVariant,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                FONT_VARIANT_CALLBACK_EAST_ASIAN_VALUE,
                value.kind as u8,
                0,
                0,
                value.value.as_bytes(),
                "",
            );
        }
    }
    if let Some(emoji) = &value.emoji {
        callback_font_variant_simple_value(callback, property_id, CssFontVariantSimpleValueKind::Emoji, Some(emoji));
    }
    if let Some(ligatures) = &value.ligatures {
        for value in ligatures {
            callback(
                CssStyleValueKind::FontVariant,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                FONT_VARIANT_CALLBACK_LIGATURES_VALUE,
                value.kind as u8,
                0,
                0,
                value.value.as_bytes(),
                "",
            );
        }
    }
    if let Some(numeric) = &value.numeric {
        for value in numeric {
            callback(
                CssStyleValueKind::FontVariant,
                property_id,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                false,
                0.0,
                FONT_VARIANT_CALLBACK_NUMERIC_VALUE,
                value.kind as u8,
                0,
                0,
                value.value.as_bytes(),
                "",
            );
        }
    }
    if let Some(position) = &value.position {
        callback_font_variant_simple_value(
            callback,
            property_id,
            CssFontVariantSimpleValueKind::Position,
            Some(position),
        );
    }
}

fn callback_font_variant_simple_value<C>(
    callback: &mut C,
    property_id: u16,
    kind: CssFontVariantSimpleValueKind,
    value: Option<&String>,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::FontVariant,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        FONT_VARIANT_CALLBACK_SIMPLE,
        kind as u8,
        0,
        0,
        value.map_or(&[], |value| value.as_bytes()),
        "",
    );
}

const POSITION_AREA_CALLBACK_NONE: u8 = 0;
const POSITION_AREA_CALLBACK_AREA: u8 = 1;
const POSITION_TRY_FALLBACK_CALLBACK_NONE: u8 = 0;
const POSITION_TRY_FALLBACK_CALLBACK_POSITION_AREA: u8 = 1;
const POSITION_TRY_FALLBACK_CALLBACK_TRY_TACTIC: u8 = 2;

fn callback_position_area_style_value<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedPositionArea,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedPositionArea::None => callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            POSITION_AREA_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedPositionArea::Area {
            first_keyword,
            second_keyword,
        } => callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            POSITION_AREA_CALLBACK_AREA,
            0,
            0,
            0,
            first_keyword.as_bytes(),
            second_keyword.as_deref().unwrap_or(""),
        ),
    }
}

fn callback_position_try_fallbacks_style_value<C>(
    callback: &mut C,
    property_id: u16,
    value: &RustOwnedPositionTryFallbacks,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    match value {
        RustOwnedPositionTryFallbacks::None => callback(
            CssStyleValueKind::PositionTryFallbacks,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            POSITION_TRY_FALLBACK_CALLBACK_NONE,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedPositionTryFallbacks::List(fallbacks) => {
            for fallback in fallbacks {
                match fallback {
                    RustOwnedPositionTryFallback::PositionArea(value) => {
                        let RustOwnedPositionArea::Area {
                            first_keyword,
                            second_keyword,
                        } = value
                        else {
                            continue;
                        };
                        callback(
                            CssStyleValueKind::PositionTryFallbacks,
                            property_id,
                            CssPrimitiveValueKind::Invalid,
                            false,
                            0.0,
                            false,
                            0.0,
                            POSITION_TRY_FALLBACK_CALLBACK_POSITION_AREA,
                            0,
                            0,
                            0,
                            first_keyword.as_bytes(),
                            second_keyword.as_deref().unwrap_or(""),
                        );
                    }
                    RustOwnedPositionTryFallback::TryTactic {
                        dashed_ident,
                        try_tactics,
                        ..
                    } => callback(
                        CssStyleValueKind::PositionTryFallbacks,
                        property_id,
                        CssPrimitiveValueKind::Invalid,
                        false,
                        0.0,
                        false,
                        0.0,
                        POSITION_TRY_FALLBACK_CALLBACK_TRY_TACTIC,
                        0,
                        0,
                        0,
                        dashed_ident.as_ref().map_or(&[], |value| value.as_bytes()),
                        &try_tactics.join(" "),
                    ),
                }
            }
        }
    }
}

const SHADOW_CALLBACK_NONE: u8 = 0;
const SHADOW_CALLBACK_BEGIN_SHADOW: u8 = 1;
const SHADOW_CALLBACK_COLOR: u8 = 2;
const SHADOW_CALLBACK_OFFSET_X: u8 = 3;
const SHADOW_CALLBACK_OFFSET_Y: u8 = 4;
const SHADOW_CALLBACK_BLUR_RADIUS: u8 = 5;
const SHADOW_CALLBACK_SPREAD_DISTANCE: u8 = 6;
const SHADOW_PLACEMENT_OUTER: u8 = 0;
const SHADOW_PLACEMENT_INNER: u8 = 1;

fn callback_shadow_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedShadow,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedShadow::None => callback_shadow_source_component(callback, property_id, SHADOW_CALLBACK_NONE, 0, ""),
        RustOwnedShadow::Shadows(shadows) => {
            for shadow in shadows {
                let placement = match shadow.placement {
                    RustOwnedShadowPlacement::Outer => SHADOW_PLACEMENT_OUTER,
                    RustOwnedShadowPlacement::Inner => SHADOW_PLACEMENT_INNER,
                };
                callback_shadow_source_component(callback, property_id, SHADOW_CALLBACK_BEGIN_SHADOW, placement, "");
                if let Some(color) = &shadow.color {
                    callback_rust_owned_color(
                        callback,
                        source_component_value_emitter,
                        CssStyleValueKind::Shadow,
                        property_id,
                        SHADOW_CALLBACK_COLOR,
                        color,
                    );
                }
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    CssStyleValueKind::Shadow,
                    property_id,
                    SHADOW_CALLBACK_OFFSET_X,
                    0,
                    &shadow.offset_x,
                );
                callback_nested_primitive_with_source_component_values_and_calculation(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    CssStyleValueKind::Shadow,
                    property_id,
                    SHADOW_CALLBACK_OFFSET_Y,
                    0,
                    &shadow.offset_y,
                );
                if let Some(blur_radius) = &shadow.blur_radius {
                    callback_nested_primitive_with_source_component_values_and_calculation(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        CssStyleValueKind::Shadow,
                        property_id,
                        SHADOW_CALLBACK_BLUR_RADIUS,
                        0,
                        blur_radius,
                    );
                }
                if let Some(spread_distance) = &shadow.spread_distance {
                    callback_nested_primitive_with_source_component_values_and_calculation(
                        callback,
                        calculation_callback,
                        source_component_value_emitter,
                        CssStyleValueKind::Shadow,
                        property_id,
                        SHADOW_CALLBACK_SPREAD_DISTANCE,
                        0,
                        spread_distance,
                    );
                }
            }
        }
    }
}

fn callback_shadow_source_component<C>(callback: &mut C, property_id: u16, component: u8, placement: u8, source: &str)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    callback(
        CssStyleValueKind::Shadow,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        component,
        placement,
        0,
        0,
        source.as_bytes(),
        "",
    );
}

fn callback_transform_longhand_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedTransformLonghand,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    match value {
        RustOwnedTransformLonghand::None => callback(
            CssStyleValueKind::TransformLonghand,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            &[],
            "",
        ),
        RustOwnedTransformLonghand::Function { function, arguments } => {
            let function = match function {
                RustOwnedTransformLonghandFunction::Rotate => TRANSFORM_LONGHAND_FUNCTION_ROTATE,
                RustOwnedTransformLonghandFunction::RotateX => TRANSFORM_LONGHAND_FUNCTION_ROTATE_X,
                RustOwnedTransformLonghandFunction::RotateY => TRANSFORM_LONGHAND_FUNCTION_ROTATE_Y,
                RustOwnedTransformLonghandFunction::RotateZ => TRANSFORM_LONGHAND_FUNCTION_ROTATE_Z,
                RustOwnedTransformLonghandFunction::Rotate3d => TRANSFORM_LONGHAND_FUNCTION_ROTATE_3D,
                RustOwnedTransformLonghandFunction::Translate => TRANSFORM_LONGHAND_FUNCTION_TRANSLATE,
                RustOwnedTransformLonghandFunction::Translate3d => TRANSFORM_LONGHAND_FUNCTION_TRANSLATE_3D,
                RustOwnedTransformLonghandFunction::Scale => TRANSFORM_LONGHAND_FUNCTION_SCALE,
                RustOwnedTransformLonghandFunction::Scale3d => TRANSFORM_LONGHAND_FUNCTION_SCALE_3D,
            };
            for (index, argument) in arguments.iter().enumerate() {
                callback_transform_function_argument(
                    callback,
                    calculation_callback,
                    source_component_value_emitter,
                    CssStyleValueKind::TransformLonghand,
                    property_id,
                    TransformFunctionArgumentCallback {
                        event_kind: TRANSFORM_LONGHAND_CALLBACK_FUNCTION,
                        function,
                        parameter_type: u8::try_from(index).expect("transform longhands have fewer than 255 arguments"),
                    },
                    argument,
                );
            }
        }
    }
}

fn callback_transformation_style_value<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    property_id: u16,
    value: &RustOwnedTransformation,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    callback(
        CssStyleValueKind::Transformation,
        property_id,
        CssPrimitiveValueKind::Invalid,
        false,
        0.0,
        false,
        0.0,
        TRANSFORMATION_CALLBACK_BEGIN_FUNCTION,
        value.function as u8,
        0,
        0,
        &[],
        "",
    );

    for argument in &value.arguments {
        callback_transform_function_argument(
            callback,
            calculation_callback,
            source_component_value_emitter,
            CssStyleValueKind::Transformation,
            property_id,
            TransformFunctionArgumentCallback {
                event_kind: TRANSFORMATION_CALLBACK_ARGUMENT,
                function: value.function as u8,
                parameter_type: argument.parameter_type as u8,
            },
            argument,
        );
    }
}

struct TransformFunctionArgumentCallback {
    event_kind: u8,
    function: u8,
    parameter_type: u8,
}

fn callback_transform_function_argument<C, D, S, E>(
    callback: &mut C,
    calculation_callback: &mut D,
    source_component_value_emitter: &mut SourceComponentValueEmitter<S, E>,
    kind: CssStyleValueKind,
    property_id: u16,
    payload: TransformFunctionArgumentCallback,
    argument: &RustOwnedTransformationArgument,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
    S: FnMut(u8),
    E: FnMut(CssComponentValue),
{
    let (primitive_kind, numeric_value, unit_or_source) = nested_primitive_callback_payload(&argument.value);
    emit_nested_primitive_source_component_values(source_component_value_emitter, &argument.value);
    callback(
        kind,
        property_id,
        primitive_kind,
        nested_primitive_callback_has_numeric_value(&argument.value),
        numeric_value,
        false,
        0.0,
        payload.event_kind,
        payload.function,
        payload.parameter_type,
        0,
        unit_or_source.as_bytes(),
        "",
    );
    if let RustOwnedNestedPrimitiveValue::MathFunction(value) = &argument.value {
        emit_rust_owned_calculation_tree(&value.calculation, calculation_callback);
    }
}

fn callback_place_shorthand_style_value<C>(
    callback: &mut C,
    kind: CssStyleValueKind,
    property_id: u16,
    value: &RustOwnedPlaceShorthand,
) where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    for keyword in &value.align_keywords {
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            keyword.as_bytes(),
            "",
        );
    }
    for keyword in &value.justify_keywords {
        callback(
            kind,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            1,
            0,
            0,
            0,
            keyword.as_bytes(),
            "",
        );
    }
}

fn callback_keyword_list_style_value<C>(callback: &mut C, property_id: u16, value: &RustOwnedKeywordList)
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    for keyword in &value.keywords {
        callback(
            CssStyleValueKind::KeywordList,
            property_id,
            CssPrimitiveValueKind::Invalid,
            false,
            0.0,
            false,
            0.0,
            0,
            0,
            0,
            0,
            keyword.as_bytes(),
            "",
        );
    }
}

fn null_separated_string_list_bytes(strings: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, string) in strings.iter().enumerate() {
        if index > 0 {
            bytes.push(0);
        }
        bytes.extend_from_slice(string.as_bytes());
    }
    bytes
}

fn null_terminated_timeline_name_item_bytes(items: &[RustOwnedTimelineNameItem]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        bytes.push(item.kind as u8);
        bytes.extend_from_slice(item.name.as_bytes());
        bytes.push(0);
    }
    bytes
}

fn null_terminated_animation_name_item_bytes(items: &[RustOwnedAnimationNameItem]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        bytes.push(item.kind as u8);
        bytes.extend_from_slice(item.value.as_bytes());
        bytes.push(0);
    }
    bytes
}

fn null_terminated_will_change_feature_bytes(features: &[RustOwnedWillChangeFeature]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for feature in features {
        bytes.push(feature.kind as u8);
        bytes.extend_from_slice(feature.value.as_bytes());
        bytes.push(0);
    }
    bytes
}
