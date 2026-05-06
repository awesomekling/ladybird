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
    CssBooleanExpressionEventKind, CssComponentValue, CssComponentValueKind, CssDeclaration, CssMediaFeature,
    CssMediaFeatureComparison, CssMediaFeatureNameKind, CssMediaFeatureSyntaxKind, CssMediaFeatureValue,
    CssMediaFeatureValueKind, CssMediaFeatureValueSyntaxKind, CssMediaQuery, CssMediaTypeKind, CssPagePseudoClassKind,
    CssPageSelector, CssRuleContext, CssRuleEvent, CssRuleEventKind, CssSyntaxNode, CssSyntaxNodeKind,
    CssValueTypeSyntaxKind,
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
