/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[path = "../../../RustAllocator.rs"]
mod rust_allocator;

mod css_parser;
mod css_tokenizer;
mod generated_media_features {
    include!(concat!(env!("OUT_DIR"), "/generated_media_features.rs"));
}
mod generated_units {
    include!(concat!(env!("OUT_DIR"), "/generated_units.rs"));
}
mod generated_value_types {
    include!(concat!(env!("OUT_DIR"), "/generated_value_types.rs"));
}

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use css_parser::{
    CssBooleanExpressionEventKind, CssColorSchemeValue, CssColorSchemeValueKind, CssComponentValue,
    CssComponentValueKind, CssContainValue, CssContainValueKind, CssContainerTypeValueKind, CssCounterStyleKind,
    CssCounterStyleNegativeSymbolCount, CssCounterStyleRangeKind, CssCounterStyleSymbolsType,
    CssCounterStyleSystemKind, CssCropOrCrossKind, CssDeclaration, CssFontFamilyValueKind, CssFontLanguageOverrideKind,
    CssFontSourceKind, CssFontStyleKind, CssFontTech, CssFontVariantAlternatesValueKind,
    CssFontVariantEastAsianValueKind, CssFontVariantLigaturesValueKind, CssFontVariantNumericValueKind,
    CssFontVariantSimpleValueKind, CssMediaFeature, CssMediaFeatureComparison, CssMediaFeatureNameKind,
    CssMediaFeatureSyntaxKind, CssMediaFeatureValue, CssMediaFeatureValueKind, CssMediaFeatureValueSyntaxKind,
    CssMediaQuery, CssMediaTypeKind, CssNonnegativeIntegerSymbolPairOrder, CssOpenTypeSettingsKind,
    CssOpenTypeTaggedValueKind, CssPagePseudoClassKind, CssPageSelector, CssRuleContext, CssRuleEvent,
    CssRuleEventKind, CssSupportsFeatureKind, CssSyntaxNode, CssSyntaxNodeKind, CssUnicodeRange,
    CssUrlCrossOriginModifierValue, CssUrlFunction, CssUrlFunctionType, CssUrlModifier, CssUrlModifierKind,
    CssUrlReferrerPolicyModifierValue, CssValueTypeSyntaxKind, CssWhiteSpaceTrimValue, CssWhiteSpaceTrimValueKind,
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
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to `callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_component_value(
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

            css_parser::parse_a_component_value(input, |component_value| {
                callback(ctx, &raw const component_value);
            });
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
            );
        });
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_supports_feature(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    feature_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssSupportsFeatureKind,
        name_ptr: *const u8,
        name_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_supports_feature(input, |kind, name| {
                let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                feature_callback(ctx, kind, name_ptr, name_len);
            })
        })
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_keyframe_selector_list(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    selector_callback: unsafe extern "C" fn(ctx: *mut c_void, selector: f64),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_keyframe_selector_list(input, |selector| {
                selector_callback(ctx, selector);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_keyframes_name(
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

            css_parser::parse_a_keyframes_name(input, |name| {
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
pub unsafe extern "C" fn rust_css_parse_custom_property_name(
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

            css_parser::parse_a_custom_property_name(input, |name| {
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
pub unsafe extern "C" fn rust_css_parse_unicode_range(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    range_callback: unsafe extern "C" fn(ctx: *mut c_void, unicode_range: *const CssUnicodeRange),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_unicode_range(input, |unicode_range| {
                range_callback(ctx, &raw const unicode_range);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_unicode_range_list(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    range_callback: unsafe extern "C" fn(ctx: *mut c_void, unicode_range: *const CssUnicodeRange),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_unicode_range_list(input, |unicode_range| {
                range_callback(ctx, &raw const unicode_range);
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
pub unsafe extern "C" fn rust_css_parse_import_url(
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

            css_parser::parse_an_import_url(
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
pub unsafe extern "C" fn rust_css_parse_font_source(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    source_callback: unsafe extern "C" fn(
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
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_source(
                input,
                |kind, family_name| {
                    let (family_name_ptr, family_name_len, family_name_is_string) = family_name
                        .map_or((std::ptr::null(), 0, false), |family_name| {
                            (family_name.name.as_ptr(), family_name.name.len(), family_name.is_string)
                        });
                    source_callback(ctx, kind, family_name_ptr, family_name_len, family_name_is_string);
                },
                |url_function| {
                    url_callback(ctx, &raw const url_function);
                },
                |modifier| {
                    modifier_callback(ctx, &raw const modifier);
                },
                |format| {
                    format_callback(ctx, format.as_ptr(), format.len());
                },
                |tech| {
                    tech_callback(ctx, tech);
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
pub unsafe extern "C" fn rust_css_parse_font_language_override(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    font_language_override_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontLanguageOverrideKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_language_override(input, |kind, value| {
                let (value_ptr, value_len) = value.map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                font_language_override_callback(ctx, kind, value_ptr, value_len);
            })
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
pub unsafe extern "C" fn rust_css_parse_container_type(
    input: *const u8,
    input_len: usize,
) -> CssContainerTypeValueKind {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssContainerTypeValueKind::Invalid;
            };

            css_parser::parse_container_type_value(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_contain(input: *const u8, input_len: usize) -> CssContainValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssContainValue {
                    kind: CssContainValueKind::Invalid,
                    is_size: false,
                    is_inline_size: false,
                    has_layout: false,
                    has_style: false,
                    has_paint: false,
                };
            };

            css_parser::parse_contain_value(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_white_space_trim(input: *const u8, input_len: usize) -> CssWhiteSpaceTrimValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssWhiteSpaceTrimValue {
                    kind: CssWhiteSpaceTrimValueKind::Invalid,
                    has_discard_before: false,
                    has_discard_after: false,
                    has_discard_inner: false,
                };
            };

            css_parser::parse_white_space_trim_value(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to `scheme_callback` must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_color_scheme(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    scheme_callback: unsafe extern "C" fn(ctx: *mut c_void, scheme_ptr: *const u8, scheme_len: usize),
) -> CssColorSchemeValue {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return CssColorSchemeValue {
                    kind: CssColorSchemeValueKind::Invalid,
                    only: false,
                };
            };

            css_parser::parse_color_scheme_value(input, |scheme| {
                scheme_callback(ctx, scheme.as_ptr(), scheme.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_feature_settings(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    settings_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssOpenTypeSettingsKind),
    tagged_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        tag_ptr: *const u8,
        tag_len: usize,
        value_kind: CssOpenTypeTaggedValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_feature_settings(
                input,
                |kind| {
                    settings_callback(ctx, kind);
                },
                |tagged_value| {
                    let (value_ptr, value_len) = tagged_value
                        .value
                        .as_ref()
                        .map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    tagged_value_callback(
                        ctx,
                        tagged_value.tag.as_ptr(),
                        tagged_value.tag.len(),
                        tagged_value.value_kind,
                        value_ptr,
                        value_len,
                    );
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
pub unsafe extern "C" fn rust_css_parse_font_variation_settings(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    settings_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssOpenTypeSettingsKind),
    tagged_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        tag_ptr: *const u8,
        tag_len: usize,
        value_kind: CssOpenTypeTaggedValueKind,
        value_ptr: *const u8,
        value_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_variation_settings(
                input,
                |kind| {
                    settings_callback(ctx, kind);
                },
                |tagged_value| {
                    let (value_ptr, value_len) = tagged_value
                        .value
                        .as_ref()
                        .map_or((std::ptr::null(), 0), |value| (value.as_ptr(), value.len()));
                    tagged_value_callback(
                        ctx,
                        tagged_value.tag.as_ptr(),
                        tagged_value.tag.len(),
                        tagged_value.value_kind,
                        value_ptr,
                        value_len,
                    );
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
pub unsafe extern "C" fn rust_css_parse_font_family_value(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    family_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        kind: CssFontFamilyValueKind,
        value_ptr: *const u8,
        value_len: usize,
        is_string: bool,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_font_family_value(input, |family_value| match family_value {
                css_parser::FontFamilyValue::Generic(value) => {
                    family_callback(ctx, CssFontFamilyValueKind::Generic, value.as_ptr(), value.len(), false);
                }
                css_parser::FontFamilyValue::FamilyName(family_name) => {
                    family_callback(
                        ctx,
                        CssFontFamilyValueKind::FamilyName,
                        family_name.name.as_ptr(),
                        family_name.name.len(),
                        family_name.is_string,
                    );
                }
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_layer_name(
    input: *const u8,
    input_len: usize,
    allow_blank_layer_name: bool,
    ctx: *mut c_void,
    name_callback: unsafe extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_layer_name(input, allow_blank_layer_name, |name| {
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
pub unsafe extern "C" fn rust_css_parse_import_layer(
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

            css_parser::parse_an_import_layer(input, |name| {
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
pub unsafe extern "C" fn rust_css_parse_layer_name_list(
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

            css_parser::parse_a_layer_name_list(input, |name| {
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
pub unsafe extern "C" fn rust_css_parse_nonnegative_integer_symbol_pair(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    order_callback: unsafe extern "C" fn(ctx: *mut c_void, order: CssNonnegativeIntegerSymbolPairOrder),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_nonnegative_integer_symbol_pair(input, |order| {
                order_callback(ctx, order);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_negative(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    count_callback: unsafe extern "C" fn(ctx: *mut c_void, count: CssCounterStyleNegativeSymbolCount),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_negative(input, |count| {
                count_callback(ctx, count);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_system(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    system_callback: unsafe extern "C" fn(ctx: *mut c_void, system: CssCounterStyleSystemKind),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_system(input, |system| {
                system_callback(ctx, system);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_symbols(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    count_callback: unsafe extern "C" fn(ctx: *mut c_void, count: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_symbols(input, |count| {
                count_callback(ctx, count);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_symbol(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_symbol(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_string_descriptor(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_string_descriptor(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_length_descriptor(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_length_descriptor(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_positive_percentage_descriptor(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_positive_percentage_descriptor(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_page_size_descriptor(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_page_size_descriptor(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_optional_declaration_value_descriptor(
    input: *const u8,
    input_len: usize,
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_optional_declaration_value_descriptor(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_range(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    range_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssCounterStyleRangeKind, count: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_range(input, |kind, count| {
                range_callback(ctx, kind, count);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_counter_style_additive_symbols(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    count_callback: unsafe extern "C" fn(ctx: *mut c_void, count: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_counter_style_additive_symbols(input, |count| {
                count_callback(ctx, count);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_crop_or_cross(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    kind_callback: unsafe extern "C" fn(ctx: *mut c_void, kind: CssCropOrCrossKind),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_crop_or_cross(input, |kind| {
                kind_callback(ctx, kind);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_font_weight_absolute_pair(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    count_callback: unsafe extern "C" fn(ctx: *mut c_void, count: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_font_weight_absolute_pair(input, |count| {
                count_callback(ctx, count);
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_namespace_rule_prelude(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    prefix_callback: unsafe extern "C" fn(ctx: *mut c_void, prefix_ptr: *const u8, prefix_len: usize),
    namespace_uri_callback: unsafe extern "C" fn(ctx: *mut c_void, uri_ptr: *const u8, uri_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_namespace_rule_prelude(
                input,
                |prefix| {
                    prefix_callback(ctx, prefix.as_ptr(), prefix.len());
                },
                |namespace_uri| {
                    namespace_uri_callback(ctx, namespace_uri.as_ptr(), namespace_uri.len());
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
pub unsafe extern "C" fn rust_css_parse_font_feature_values_family_name_list(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    family_name_callback: unsafe extern "C" fn(ctx: *mut c_void, family_name_ptr: *const u8, family_name_len: usize),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_font_feature_values_family_name_list(input, |family_name| {
                family_name_callback(ctx, family_name.as_ptr(), family_name.len());
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_empty_prelude(input: *const u8, input_len: usize) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_empty_prelude(input)
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers when their
///   corresponding `has_*` parameter is true
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_container_rule_prelude(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    condition_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        has_name: bool,
        name_ptr: *const u8,
        name_len: usize,
        has_query: bool,
        query_ptr: *const u8,
        query_len: usize,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_container_rule_prelude(input, |name, query| {
                let (name_ptr, name_len) = name.map_or((std::ptr::null(), 0), |name| (name.as_ptr(), name.len()));
                let (query_ptr, query_len) = query.map_or((std::ptr::null(), 0), |query| (query.as_ptr(), query.len()));
                condition_callback(
                    ctx,
                    name.is_some(),
                    name_ptr,
                    name_len,
                    query.is_some(),
                    query_ptr,
                    query_len,
                );
            })
        })
    }
}

/// # Safety
/// - `input` and `input_len` must point to a valid string
/// - `ctx` must be a valid pointer to a CallbackContext
/// - Parameters provided to callbacks must be valid pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_css_parse_family_name(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    family_name_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        family_name_ptr: *const u8,
        family_name_len: usize,
        is_string: bool,
    ),
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_family_name(input, |family_name, is_string| {
                family_name_callback(ctx, family_name.as_ptr(), family_name.len(), is_string);
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
pub unsafe extern "C" fn rust_css_parse_media_test(
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

            css_parser::parse_a_media_test(
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
pub unsafe extern "C" fn rust_css_parse_media_query(
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
) -> bool {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return false;
            };

            css_parser::parse_a_media_query(
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
            )
        })
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
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
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
pub unsafe extern "C" fn rust_css_parse_block_contents(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    event_callback: unsafe extern "C" fn(ctx: *mut c_void, event: *const CssRuleEvent),
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_blocks_contents(
                input,
                |event| {
                    event_callback(ctx, &raw const event);
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

            css_parser::parse_a_blocks_contents_with_context(
                input,
                rule_context,
                |event| {
                    event_callback(ctx, &raw const event);
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
    component_value_callback: unsafe extern "C" fn(ctx: *mut c_void, component_value: *const CssComponentValue),
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
                |component_value| {
                    component_value_callback(ctx, &raw const component_value);
                },
            );
        });
    }
}
