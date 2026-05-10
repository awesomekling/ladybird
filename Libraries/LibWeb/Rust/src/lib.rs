/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[path = "../../../RustAllocator.rs"]
mod rust_allocator;

mod css_parser;
mod css_tokenizer;
mod generated_descriptors {
    include!(concat!(env!("OUT_DIR"), "/generated_descriptors.rs"));
}
mod generated_media_features {
    include!(concat!(env!("OUT_DIR"), "/generated_media_features.rs"));
}
mod generated_properties {
    include!(concat!(env!("OUT_DIR"), "/generated_properties.rs"));
}
#[allow(dead_code)]
mod generated_pseudo_classes {
    include!(concat!(env!("OUT_DIR"), "/generated_pseudo_classes.rs"));
}
#[allow(dead_code)]
mod generated_pseudo_elements {
    include!(concat!(env!("OUT_DIR"), "/generated_pseudo_elements.rs"));
}
mod generated_units {
    include!(concat!(env!("OUT_DIR"), "/generated_units.rs"));
}
mod generated_value_types {
    include!(concat!(env!("OUT_DIR"), "/generated_value_types.rs"));
}
mod generated_transform_functions {
    include!(concat!(env!("OUT_DIR"), "/generated_transform_functions.rs"));
}

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use css_parser::{
    CssAnchorNameOrScopeValueKind, CssAnimationNameItemKind, CssAnimationNameValueKind, CssAttributeCaseType,
    CssAttributeMatchType, CssBackgroundSizeValueKind, CssBasicShapeValueKind, CssBooleanExpressionEventKind,
    CssCalculationNodeKind, CssColorFunctionValueKind, CssColorSchemeValue, CssColorSchemeValueKind, CssColorValueKind,
    CssComponentValue, CssComponentValueKind, CssContainValue, CssContainValueKind, CssContainerTypeValueKind,
    CssCounterStyleKind, CssCounterStyleNegativeSymbolCount, CssCounterStyleRangeKind, CssCounterStyleSymbolsType,
    CssCounterStyleSystemKind, CssCropOrCrossKind, CssDeclaration, CssDescriptorResultKind, CssDescriptorSyntaxKind,
    CssDescriptorValueType, CssEasingValueKind, CssFitContentValueKind, CssFontFamilyValueKind,
    CssFontLanguageOverrideKind, CssFontSourceKind, CssFontStyleKind, CssFontTech, CssFontVariantAlternatesValueKind,
    CssFontVariantEastAsianValueKind, CssFontVariantLigaturesValueKind, CssFontVariantNumericValueKind,
    CssFontVariantSimpleValueKind, CssGridAutoFlowValueKind, CssGridTrackPlacementValueKind,
    CssGridTrackSizeListValueKind, CssImageSetValueKind, CssMediaFeature, CssMediaFeatureComparison,
    CssMediaFeatureNameKind, CssMediaFeatureSyntaxKind, CssMediaFeatureValue, CssMediaFeatureValueKind,
    CssMediaFeatureValueSyntaxKind, CssMediaQuery, CssMediaTypeKind, CssNonnegativeIntegerSymbolPairOrder,
    CssOpenTypeSettingsKind, CssOpenTypeTaggedValueKind, CssPagePseudoClassKind, CssPageSelector,
    CssPageSizeDescriptorKind, CssPaintOrderKeyword, CssPaintOrderValue, CssPaintOrderValueKind, CssParsedColorKind,
    CssPositionAnchorValueKind, CssPositionTryOrderValue, CssPositionValueKind, CssPositionVisibilityValue,
    CssPositionVisibilityValueKind, CssPrimitiveValueKind, CssPrimitiveValueOptions, CssPrimitiveValueType,
    CssPseudoElementValueKind, CssQuotesValueKind, CssRatioValue, CssRatioValueKind, CssRectValueKind,
    CssRepeatStyleValueKind, CssRuleContext, CssRuleEvent, CssRuleEventKind, CssScrollFunctionAxisKind,
    CssScrollFunctionScrollerKind, CssScrollFunctionValue, CssScrollFunctionValueKind, CssScrollbarGutterValueKind,
    CssSelectorCombinator, CssSelectorEvent, CssSelectorEventKind, CssSelectorNamespace, CssSelectorNamespaceType,
    CssSimpleSelectorKind, CssStyleValueKind, CssSupportsFeatureKind, CssSyntaxNode, CssSyntaxNodeKind,
    CssTextUnderlinePositionHorizontal, CssTextUnderlinePositionValue, CssTextUnderlinePositionVertical,
    CssTextWrapModeValue, CssTextWrapStyleValue, CssTextWrapValue, CssTextWrapValueKind, CssTimelineNameItemKind,
    CssTimelineNameValueKind, CssTimelineScopeValueKind, CssTouchActionKeyword, CssTouchActionValue,
    CssTouchActionValueKind, CssTransformFunctionValueKind, CssTransformLonghandValueKind,
    CssTransitionBehaviorItemKind, CssTransitionBehaviorValueKind, CssTransitionPropertyValueKind, CssUnicodeRange,
    CssUrlCrossOriginModifierValue, CssUrlFunction, CssUrlFunctionType, CssUrlModifier, CssUrlModifierKind,
    CssUrlReferrerPolicyModifierValue, CssValueTypeSyntaxKind, CssViewFunctionInsetKind, CssViewFunctionInsetPosition,
    CssViewFunctionValue, CssViewFunctionValueKind, CssViewTimelineInsetValue, CssViewTimelineInsetValueKind,
    CssViewTransitionNameValueKind, CssWhiteSpaceTrimValue, CssWhiteSpaceTrimValueKind, CssWillChangeFeatureKind,
    CssWillChangeValueKind,
};
pub use css_tokenizer::{CssHashType, CssNumberType, CssToken, CssTokenType};

fn abort_on_panic<F: FnOnce() -> R, R>(f: F) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("Rust panic at FFI boundary: {message}");
            std::process::abort();
        }
    }
}

unsafe fn bytes_from_raw<'a>(bytes: *const u8, len: usize) -> Option<&'a [u8]> {
    unsafe {
        if len == 0 {
            return Some(&[]);
        }
        if bytes.is_null() {
            eprintln!("bytes_from_raw: null pointer with non-zero length {len}");
            return None;
        }
        Some(std::slice::from_raw_parts(bytes, len))
    }
}

unsafe fn slice_from_raw<'a, T>(items: *const T, len: usize) -> Option<&'a [T]> {
    unsafe {
        if len == 0 {
            return Some(&[]);
        }
        if items.is_null() {
            eprintln!("slice_from_raw: null pointer with non-zero length {len}");
            return None;
        }
        Some(std::slice::from_raw_parts(items, len))
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_tokenize(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(ctx: *mut c_void, token: *const CssToken),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_tokenizer::tokenize(input, |token, filtered_input| {
                let ffi_token = token.as_ffi(filtered_input);
                callback(ctx, &raw const ffi_token);
            });
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_component_values(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_list_of_component_values(input, |component_value| {
                callback(ctx, &raw const component_value);
            });
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `declared_namespaces` and `declared_namespaces_len` must point to a valid slice
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_selector_list(
    input: *const u8,
    input_len: usize,
    selector_type: u8,
    parsing_mode: u8,
    declared_namespaces: *const CssSelectorNamespace,
    declared_namespaces_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssSelectorEvent),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };
            let Some(declared_namespaces) = slice_from_raw(declared_namespaces, declared_namespaces_len) else {
                return false;
            };

            let selector_type = match selector_type {
                0 => css_parser::SelectorType::Standalone,
                1 => css_parser::SelectorType::Relative,
                _ => return false,
            };
            let parsing_mode = match parsing_mode {
                0 => css_parser::SelectorParsingMode::Normal,
                1 => css_parser::SelectorParsingMode::Forgiving,
                _ => return false,
            };
            let declared_namespaces = declared_namespaces
                .iter()
                .map(|namespace| {
                    bytes_from_raw(namespace.prefix_ptr, namespace.prefix_len)
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .map(str::to_string)
                })
                .collect::<Option<Vec<_>>>();
            let Some(declared_namespaces) = declared_namespaces else {
                return false;
            };

            css_parser::parse_a_selector_list(
                input,
                selector_type,
                parsing_mode,
                declared_namespaces,
                |event| {
                    event_callback(ctx, &raw const event);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_variant_alternates(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    value_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssFontVariantAlternatesValueKind),
    feature_value_name_callback: unsafe extern "C" fn(ctx: *mut c_void, value_ptr: *const u8, value_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variant_alternates(
                input,
                |kind| {
                    value_callback(ctx, kind);
                },
                |feature_value_name| {
                    feature_value_name_callback(ctx, feature_value_name.as_ptr(), feature_value_name.len());
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_variant(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    simple_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantSimpleValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
    alternates_value_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssFontVariantAlternatesValueKind),
    alternates_feature_value_name_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        value_ptr: *const u8,
        value_len: usize,
    ),
    east_asian_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantEastAsianValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
    numeric_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantNumericValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
    ligatures_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantLigaturesValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variant(
                input,
                |kind, value| {
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    simple_value_callback(ctx, kind, value_ptr, value_len);
                },
                |kind| {
                    alternates_value_callback(ctx, kind);
                },
                |feature_value_name| {
                    alternates_feature_value_name_callback(ctx, feature_value_name.as_ptr(), feature_value_name.len());
                },
                |value| {
                    east_asian_value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
                },
                |value| {
                    numeric_value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
                },
                |value| {
                    ligatures_value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_comma_separated_component_values(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    group_callback: unsafe extern "C" fn(ctx: *mut c_void),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_comma_separated_list_of_component_values(
                input,
                || {
                    group_callback(ctx);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_value_type(
    input: *const u8,
    input_len: usize,
    value_type_id: u8,
) -> CssValueTypeSyntaxKind {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssValueTypeSyntaxKind::Invalid;
            };

            css_parser::parse_a_value_type(input, value_type_id)
        })
    }
}

/// # Safety
/// - `property_ids` and `property_ids_len` must point to valid PropertyID values
/// - `keyword` and `keyword_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_property_keyword_value(
    property_ids: *const u16,
    property_ids_len: usize,
    keyword: *const u8,
    keyword_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(ctx: *mut c_void, property_id: u16, keyword: *const u8, keyword_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(property_ids) = slice_from_raw(property_ids, property_ids_len) else {
                return false;
            };
            let Some(keyword) = bytes_from_raw(keyword, keyword_len) else {
                return false;
            };

            css_parser::parse_property_keyword_value(property_ids, keyword, |property_id, keyword| {
                callback(ctx, property_id, keyword.as_ptr(), keyword.len());
            })
        })
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_css_at_rule_supports_descriptor(at_rule_id: u8, descriptor_id: u8) -> bool {
    abort_on_panic(|| css_parser::descriptor_supports(at_rule_id, descriptor_id))
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_css_descriptor_allows_arbitrary_substitution_functions(
    at_rule_id: u8,
    descriptor_id: u8,
) -> bool {
    abort_on_panic(|| css_parser::descriptor_allows_arbitrary_substitution_functions(at_rule_id, descriptor_id))
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_descriptor(
    at_rule_id: u8,
    descriptor_id: u8,
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    syntax_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssDescriptorSyntaxKind,
        property_id: u16,
        value_type: CssDescriptorValueType,
        value: *const u8,
        value_len: usize,
    ),
    kind_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssDescriptorResultKind),
    source_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        order: CssNonnegativeIntegerSymbolPairOrder,
        source: *const u8,
        source_len: usize,
        is_string: bool,
        primitive_kind: CssPrimitiveValueKind,
        has_numeric_value: bool,
        numeric_value: f64,
        page_size_keyword: u8,
        page_size_orientation: u8,
    ),
    calculation_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssCalculationNodeKind,
        primitive_kind: CssPrimitiveValueKind,
        has_numeric_value: bool,
        numeric_value: f64,
        child_count: u32,
        metadata_ptr: *const u8,
        metadata_len: usize,
    ),
    font_source_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontSourceKind,
        family_name_ptr: *const u8,
        family_name_len: usize,
        family_name_is_string: bool,
    ),
    url_callback: unsafe extern "C" fn(ctx: *mut c_void, url_function: *const CssUrlFunction),
    modifier_callback: unsafe extern "C" fn(ctx: *mut c_void, modifier: *const CssUrlModifier),
    format_callback: unsafe extern "C" fn(ctx: *mut c_void, format_ptr: *const u8, format_len: usize),
    tech_callback: unsafe extern "C" fn(ctx: *mut c_void, tech: CssFontTech),
    unicode_range_callback: unsafe extern "C" fn(ctx: *mut c_void, unicode_range: *const CssUnicodeRange),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_descriptor(
                at_rule_id,
                descriptor_id,
                input,
                |kind, property_id, value_type, value| {
                    syntax_callback(ctx, kind, property_id, value_type, value.as_ptr(), value.len());
                },
                css_parser::DescriptorResultCallbacks {
                    kind_callback: |kind| kind_callback(ctx, kind),
                    source_callback: |order,
                                      source: &str,
                                      is_string,
                                      primitive_kind,
                                      has_numeric_value,
                                      numeric_value,
                                      page_size_keyword,
                                      page_size_orientation| {
                        source_callback(
                            ctx,
                            order,
                            source.as_ptr(),
                            source.len(),
                            is_string,
                            primitive_kind,
                            has_numeric_value,
                            numeric_value,
                            page_size_keyword,
                            page_size_orientation,
                        );
                    },
                    calculation_callback: |kind,
                                           primitive_kind,
                                           has_numeric_value,
                                           numeric_value,
                                           child_count,
                                           metadata: &[u8]| {
                        calculation_callback(
                            ctx,
                            kind,
                            primitive_kind,
                            has_numeric_value,
                            numeric_value,
                            child_count,
                            metadata.as_ptr(),
                            metadata.len(),
                        );
                    },
                    font_source_callback: |kind, family_name: Option<&str>, family_name_is_string| {
                        let (family_name_ptr, family_name_len) = family_name
                            .map_or((std::ptr::null(), 0), |family_name| {
                                (family_name.as_ptr(), family_name.len())
                            });
                        font_source_callback(ctx, kind, family_name_ptr, family_name_len, family_name_is_string);
                    },
                    url_callback: |url_function| {
                        url_callback(ctx, &raw const url_function);
                    },
                    modifier_callback: |modifier| {
                        modifier_callback(ctx, &raw const modifier);
                    },
                    format_callback: |format: &str| {
                        format_callback(ctx, format.as_ptr(), format.len());
                    },
                    tech_callback: |tech| {
                        tech_callback(ctx, tech);
                    },
                    unicode_range_callback: |unicode_range| {
                        unicode_range_callback(ctx, &raw const unicode_range);
                    },
                },
            )
        })
    }
}

/// # Safety
/// - `property_ids` and `property_ids_len` must point to valid PropertyID values
/// - `value_type` and `value_type_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_property_accepting_type(
    property_ids: *const u16,
    property_ids_len: usize,
    value_type: *const u8,
    value_type_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(ctx: *mut c_void, property_id: u16),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(property_ids) = slice_from_raw(property_ids, property_ids_len) else {
                return false;
            };
            let Some(value_type) = bytes_from_raw(value_type, value_type_len) else {
                return false;
            };

            css_parser::property_accepting_type(property_ids, value_type, |property_id| {
                callback(ctx, property_id);
            })
        })
    }
}

/// # Safety
/// - `property_ids` and `property_ids_len` must point to valid PropertyID values
/// - `input` and `input_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_property_custom_ident_value(
    property_ids: *const u16,
    property_ids_len: usize,
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        property_id: u16,
        custom_ident: *const u8,
        custom_ident_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(property_ids) = slice_from_raw(property_ids, property_ids_len) else {
                return false;
            };
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_property_custom_ident_value(property_ids, input, |property_id, custom_ident| {
                callback(ctx, property_id, custom_ident.as_ptr(), custom_ident.len());
            })
        })
    }
}

/// # Safety
/// - `property_ids` and `property_ids_len` must point to valid PropertyID values
/// - `input` and `input_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_style_value_for_property(
    property_ids: *const u16,
    property_ids_len: usize,
    input: *const u8,
    input_len: usize,
    allow_quirky_length: bool,
    allow_quirky_color: bool,
    allow_svg_unitless_length: bool,
    allow_svg_unitless_angle: bool,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssStyleValueKind,
        property_id: u16,
        primitive_kind: CssPrimitiveValueKind,
        has_numeric_value: bool,
        numeric_value: f64,
        has_secondary_numeric_value: bool,
        secondary_numeric_value: f64,
        color_red: u8,
        color_green: u8,
        color_blue: u8,
        color_alpha: u8,
        value: *const u8,
        value_len: usize,
        value_type: *const u8,
        value_type_len: usize,
    ),
    calculation_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssCalculationNodeKind,
        primitive_kind: CssPrimitiveValueKind,
        has_numeric_value: bool,
        numeric_value: f64,
        child_count: u32,
        metadata: *const u8,
        metadata_len: usize,
    ),
    url_modifier_callback: unsafe extern "C" fn(ctx: *mut c_void, modifier: *const CssUrlModifier),
    source_component_value_list_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: u8),
    source_component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(property_ids) = slice_from_raw(property_ids, property_ids_len) else {
                return false;
            };
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_style_value_for_property_with_options_and_calculation_callback(
                property_ids,
                input,
                css_parser::CssPrimitiveValueOptions {
                    allow_quirky_length,
                    allow_quirky_color,
                    allow_svg_unitless_length,
                    allow_svg_unitless_angle,
                },
                |kind,
                 property_id,
                 primitive_kind,
                 has_numeric_value,
                 numeric_value,
                 has_secondary_numeric_value,
                 secondary_numeric_value,
                 color_red,
                 color_green,
                 color_blue,
                 color_alpha,
                 value,
                 value_type| {
                    callback(
                        ctx,
                        kind,
                        property_id,
                        primitive_kind,
                        has_numeric_value,
                        numeric_value,
                        has_secondary_numeric_value,
                        secondary_numeric_value,
                        color_red,
                        color_green,
                        color_blue,
                        color_alpha,
                        value.as_ptr(),
                        value.len(),
                        value_type.as_ptr(),
                        value_type.len(),
                    );
                },
                |kind, primitive_kind, has_numeric_value, numeric_value, child_count, metadata| {
                    calculation_callback(
                        ctx,
                        kind,
                        primitive_kind,
                        has_numeric_value,
                        numeric_value,
                        child_count,
                        metadata.as_ptr(),
                        metadata.len(),
                    );
                },
                |modifier| {
                    let modifier = modifier.as_ffi();
                    url_modifier_callback(ctx, &raw const modifier);
                },
                (
                    &mut |kind| {
                        source_component_value_list_callback(ctx, kind);
                    },
                    &mut |component_value| {
                        source_component_value_callback(ctx, &raw const component_value);
                    },
                ),
            )
        })
    }
}

/// # Safety
/// - `property_ids` and `property_ids_len` must point to valid PropertyID values
/// - `value_type` and `value_type_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_property_numeric_metadata(
    property_ids: *const u16,
    property_ids_len: usize,
    value_type: *const u8,
    value_type_len: usize,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        property_id: u16,
        minimum: f64,
        maximum: f64,
        has_percentage_range: bool,
        percentage_minimum: f64,
        percentage_maximum: f64,
        percentages_resolve_to_value_type: bool,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(property_ids) = slice_from_raw(property_ids, property_ids_len) else {
                return false;
            };
            let Some(value_type) = bytes_from_raw(value_type, value_type_len) else {
                return false;
            };

            css_parser::property_numeric_metadata(
                property_ids,
                value_type,
                |property_id,
                 minimum,
                 maximum,
                 has_percentage_range,
                 percentage_minimum,
                 percentage_maximum,
                 percentages_resolve_to_value_type| {
                    callback(
                        ctx,
                        property_id,
                        minimum,
                        maximum,
                        has_percentage_range,
                        percentage_minimum,
                        percentage_maximum,
                        percentages_resolve_to_value_type,
                    );
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_as_syntax(
    input: *const u8,
    input_len: usize,
    limit_single_component_ident_to_custom_ident: bool,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(ctx: *mut c_void, syntax_node: *const CssSyntaxNode),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_as_syntax(input, limit_single_component_ident_to_custom_ident, |syntax_node| {
                callback(ctx, &raw const syntax_node);
            });
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_supports_condition(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    supports_feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
        value_ptr: *const u8,
        value_len: usize,
        important: bool,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_supports_condition(
                input,
                |event| {
                    event_callback(ctx, event);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
                |kind, name, value, important| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    supports_feature_callback(ctx, kind, name_ptr, name_len, value_ptr, value_len, important);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_if_condition(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    supports_feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
        value_ptr: *const u8,
        value_len: usize,
        important: bool,
    ),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_an_if_condition(
                input,
                |event| {
                    event_callback(ctx, event);
                },
                |kind, name, value, important| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    supports_feature_callback(ctx, kind, name_ptr, name_len, value_ptr, value_len, important);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_arbitrary_substitution_function_declaration_value_arguments(
    input: *const u8,
    input_len: usize,
    function: u8,
    ctx: *mut c_void,
    group_callback: unsafe extern "C" fn(ctx: *mut c_void),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };
            let filtered_input = std::str::from_utf8(input).expect("rust_css_parse_* received non-UTF-8 input");

            let Some(groups) =
                css_parser::parse_arbitrary_substitution_function_declaration_value_arguments(input, function)
            else {
                return false;
            };

            for group in groups {
                group_callback(ctx);
                css_parser::emit_component_values(&group, filtered_input, &mut |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                });
            }

            true
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_arbitrary_substitution_function_if_arguments(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    branch_callback: unsafe extern "C" fn(ctx: *mut c_void),
    condition_end_callback: unsafe extern "C" fn(ctx: *mut c_void),
    branch_end_callback: unsafe extern "C" fn(ctx: *mut c_void),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };
            let filtered_input = std::str::from_utf8(input).expect("rust_css_parse_* received non-UTF-8 input");

            let Some(branches) = css_parser::parse_arbitrary_substitution_function_if_arguments(input) else {
                return false;
            };

            for branch in branches {
                branch_callback(ctx);
                css_parser::emit_component_values(&branch.condition, filtered_input, &mut |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                });
                condition_end_callback(ctx);
                css_parser::emit_component_values(&branch.value, filtered_input, &mut |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                });
                branch_end_callback(ctx);
            }

            true
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_page_selector_list(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    selector_callback: unsafe extern "C" fn(ctx: *mut c_void, selector: *const CssPageSelector),
    pseudo_class_callback: unsafe extern "C" fn(ctx: *mut c_void, pseudo_class: CssPagePseudoClassKind),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_page_selector_list(
                input,
                |selector| {
                    selector_callback(ctx, &raw const selector);
                },
                |pseudo_class| {
                    pseudo_class_callback(ctx, pseudo_class);
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_custom_ident(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    name_callback: unsafe extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_custom_ident(input, |name| {
                name_callback(ctx, name.as_ptr(), name.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_dashed_ident(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    name_callback: unsafe extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_dashed_ident(input, |name| {
                name_callback(ctx, name.as_ptr(), name.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_url_function(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    url_callback: unsafe extern "C" fn(ctx: *mut c_void, url_function: *const CssUrlFunction),
    modifier_callback: unsafe extern "C" fn(ctx: *mut c_void, modifier: *const CssUrlModifier),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_url_function(
                input,
                |url_function| {
                    url_callback(ctx, &raw const url_function);
                },
                |modifier| {
                    modifier_callback(ctx, &raw const modifier);
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_opentype_tag(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    opentype_tag_callback: unsafe extern "C" fn(ctx: *mut c_void, value_ptr: *const u8, value_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_an_opentype_tag(input, |value| {
                opentype_tag_callback(ctx, value.as_ptr(), value.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_scroll_function(input: *const u8, input_len: usize) -> CssScrollFunctionValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssScrollFunctionValue {
                    kind: CssScrollFunctionValueKind::Invalid,
                    scroller: CssScrollFunctionScrollerKind::None,
                    axis: CssScrollFunctionAxisKind::None,
                };
            };

            css_parser::parse_scroll_function_value(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_view_timeline_inset_prefix(
    input: *const u8,
    input_len: usize,
) -> CssViewTimelineInsetValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssViewTimelineInsetValue {
                    kind: CssViewTimelineInsetValueKind::Invalid,
                    count: 0,
                };
            };

            css_parser::parse_view_timeline_inset_value_prefix(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_view_function(input: *const u8, input_len: usize) -> CssViewFunctionValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssViewFunctionValue {
                    kind: CssViewFunctionValueKind::Invalid,
                    axis: CssScrollFunctionAxisKind::None,
                    inset: CssViewFunctionInsetKind::None,
                    inset_position: CssViewFunctionInsetPosition::None,
                };
            };

            css_parser::parse_view_function_value(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_primitive_value_prefix(
    input: *const u8,
    input_len: usize,
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssPrimitiveValueKind::Invalid;
            };

            css_parser::parse_primitive_value_prefix(input, value_type, options)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_primitive_value(
    input: *const u8,
    input_len: usize,
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssPrimitiveValueKind::Invalid;
            };

            css_parser::parse_primitive_value(input, value_type, options)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_color(
    input: *const u8,
    input_len: usize,
    allow_quirky_color: bool,
) -> CssColorValueKind {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssColorValueKind::Invalid;
            };

            css_parser::parse_color_value(input, allow_quirky_color)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_simple_color(
    input: *const u8,
    input_len: usize,
    allow_quirky_color: bool,
    ctx: *mut c_void,
    callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssParsedColorKind,
        red: u8,
        green: u8,
        blue: u8,
        alpha: u8,
        name: *const u8,
        name_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_simple_color_value(input, allow_quirky_color, |kind, red, green, blue, alpha, name| {
                callback(ctx, kind, red, green, blue, alpha, name.as_ptr(), name.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_style(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    font_style_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssFontStyleKind, has_angle: bool),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_style(input, |font_style| {
                let (kind, has_angle) = match font_style {
                    css_parser::FontStyle::Normal => (CssFontStyleKind::Normal, false),
                    css_parser::FontStyle::Italic => (CssFontStyleKind::Italic, false),
                    css_parser::FontStyle::Left => (CssFontStyleKind::Left, false),
                    css_parser::FontStyle::Right => (CssFontStyleKind::Right, false),
                    css_parser::FontStyle::Oblique { has_angle } => (CssFontStyleKind::Oblique, has_angle),
                };
                font_style_callback(ctx, kind, has_angle);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_variant_east_asian(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantEastAsianValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variant_east_asian(input, |value| {
                value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_variant_numeric(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantNumericValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variant_numeric(input, |value| {
                value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_variant_ligatures(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontVariantLigaturesValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variant_ligatures(input, |value| {
                value_callback(ctx, value.kind, value.value.as_ptr(), value.value.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_name(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    name_callback: unsafe extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_counter_style_name(input, |name| {
                name_callback(ctx, name.as_ptr(), name.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    counter_style_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssCounterStyleKind,
        symbols_type: CssCounterStyleSymbolsType,
        name_ptr: *const u8,
        name_len: usize,
    ),
    symbol_callback: unsafe extern "C" fn(ctx: *mut c_void, symbol_ptr: *const u8, symbol_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_counter_style(
                input,
                |kind, symbols_type, name| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    counter_style_callback(ctx, kind, symbols_type, name_ptr, name_len);
                },
                |symbol| {
                    symbol_callback(ctx, symbol.as_ptr(), symbol.len());
                },
            )
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    counter_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        function: u8,
        name_ptr: *const u8,
        name_len: usize,
        join_string_ptr: *const u8,
        join_string_len: usize,
    ),
    counter_style_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssCounterStyleKind,
        symbols_type: CssCounterStyleSymbolsType,
        name_ptr: *const u8,
        name_len: usize,
    ),
    symbol_callback: unsafe extern "C" fn(ctx: *mut c_void, symbol_ptr: *const u8, symbol_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            let Some(counter) = css_parser::parse_rust_owned_counter_function(input) else {
                return false;
            };

            let function = match counter.function {
                css_parser::RustOwnedCounterFunctionKind::Counter => 0,
                css_parser::RustOwnedCounterFunctionKind::Counters => 1,
            };
            let (join_string_ptr, join_string_len) = counter
                .join_string
                .as_ref()
                .map_or((std::ptr::null(), 0), |join_string| {
                    (join_string.as_ptr(), join_string.len())
                });
            counter_callback(
                ctx,
                function,
                counter.counter_name.as_ptr(),
                counter.counter_name.len(),
                join_string_ptr,
                join_string_len,
            );

            if let Some(counter_style) = counter.counter_style.as_ref() {
                match counter_style {
                    css_parser::CounterStyle::Name(name) => counter_style_callback(
                        ctx,
                        CssCounterStyleKind::Name,
                        CssCounterStyleSymbolsType::Symbolic,
                        name.as_ptr(),
                        name.len(),
                    ),
                    css_parser::CounterStyle::SymbolsFunction { symbols_type, symbols } => {
                        counter_style_callback(
                            ctx,
                            CssCounterStyleKind::SymbolsFunction,
                            *symbols_type,
                            std::ptr::null(),
                            0,
                        );
                        for symbol in symbols {
                            symbol_callback(ctx, symbol.as_ptr(), symbol.len());
                        }
                    }
                }
            }

            true
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_feature_values_feature_value(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    value_callback: unsafe extern "C" fn(ctx: *mut c_void, value: u32),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_font_feature_values_feature_value(input, |value| {
                value_callback(ctx, value);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_media_condition(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_media_condition(
                input,
                |event| {
                    event_callback(ctx, event);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_media_query_list(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    media_query_callback: unsafe extern "C" fn(ctx: *mut c_void, media_query: *const CssMediaQuery),
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_media_query_list(
                input,
                |media_query| {
                    media_query_callback(ctx, &raw const media_query);
                },
                |event| {
                    event_callback(ctx, event);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_declaration(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    declaration_callback: unsafe extern "C" fn(ctx: *mut c_void, declaration: *const CssDeclaration),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_declaration(
                input,
                |declaration| {
                    declaration_callback(ctx, &raw const declaration);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `rule_context` and `rule_context_len` must point to a valid rule context slice
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_declaration_with_context(
    input: *const u8,
    input_len: usize,
    rule_context: *const CssRuleContext,
    rule_context_len: usize,
    ctx: *mut c_void,
    declaration_callback: unsafe extern "C" fn(ctx: *mut c_void, declaration: *const CssDeclaration),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };
            let Some(rule_context) = slice_from_raw(rule_context, rule_context_len) else {
                return;
            };

            css_parser::parse_a_declaration_with_context(
                input,
                rule_context,
                |declaration| {
                    declaration_callback(ctx, &raw const declaration);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_rule(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssRuleEvent),
    media_query_callback: unsafe extern "C" fn(ctx: *mut c_void, media_query: *const CssMediaQuery),
    boolean_expression_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    supports_feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
        value_ptr: *const u8,
        value_len: usize,
        important: bool,
    ),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
    selector_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssSelectorEvent),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_rule(
                input,
                |event| {
                    event_callback(ctx, &raw const event);
                },
                |event| {
                    selector_event_callback(ctx, &raw const event);
                },
                |media_query| {
                    media_query_callback(ctx, &raw const media_query);
                },
                |event| {
                    boolean_expression_event_callback(ctx, event);
                },
                |kind, name, value, important| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    supports_feature_callback(ctx, kind, name_ptr, name_len, value_ptr, value_len, important);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `rule_context` and `rule_context_len` must point to a valid rule context slice
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_block_contents_with_context(
    input: *const u8,
    input_len: usize,
    rule_context: *const CssRuleContext,
    rule_context_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssRuleEvent),
    media_query_callback: unsafe extern "C" fn(ctx: *mut c_void, media_query: *const CssMediaQuery),
    boolean_expression_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    supports_feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
        value_ptr: *const u8,
        value_len: usize,
        important: bool,
    ),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
    selector_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssSelectorEvent),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };
            let Some(rule_context) = slice_from_raw(rule_context, rule_context_len) else {
                return;
            };

            css_parser::parse_a_blocks_contents_with_context(
                input,
                rule_context,
                |event| {
                    event_callback(ctx, &raw const event);
                },
                |event| {
                    selector_event_callback(ctx, &raw const event);
                },
                |media_query| {
                    media_query_callback(ctx, &raw const media_query);
                },
                |event| {
                    boolean_expression_event_callback(ctx, event);
                },
                |kind, name, value, important| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    supports_feature_callback(ctx, kind, name_ptr, name_len, value_ptr, value_len, important);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_stylesheet_contents(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssRuleEvent),
    media_query_callback: unsafe extern "C" fn(ctx: *mut c_void, media_query: *const CssMediaQuery),
    boolean_expression_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: CssBooleanExpressionEventKind),
    supports_feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
        value_ptr: *const u8,
        value_len: usize,
        important: bool,
    ),
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
    selector_event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssSelectorEvent),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_stylesheets_contents(
                input,
                |event| {
                    event_callback(ctx, &raw const event);
                },
                |event| {
                    selector_event_callback(ctx, &raw const event);
                },
                |media_query| {
                    media_query_callback(ctx, &raw const media_query);
                },
                |event| {
                    boolean_expression_event_callback(ctx, event);
                },
                |kind, name, value, important| {
                    let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                    let (value_ptr, value_len) =
                        value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    supports_feature_callback(ctx, kind, name_ptr, name_len, value_ptr, value_len, important);
                },
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
                },
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}
