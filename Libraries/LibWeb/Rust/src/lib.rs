/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[path = "../../../RustAllocator.rs"]
mod rust_allocator;

mod css_parser;
mod css_tokenizer;
#[allow(dead_code)]
mod generated_media_features {
    include!(concat!(env!("OUT_DIR"), "/generated_media_features.rs"));
}

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use css_parser::{
    CssBooleanExpressionEventKind, CssComponentValue, CssComponentValueKind, CssDeclaration, CssMediaFeature,
    CssMediaFeatureComparison, CssMediaFeatureNameKind, CssMediaFeatureSyntaxKind, CssMediaFeatureValue,
    CssMediaFeatureValueKind, CssRuleContext, CssRuleEvent, CssRuleEventKind,
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
pub unsafe extern "C" fn rust_css_parse_media_feature(
    input: *const u8,
    input_len: usize,
    ctx: *mut c_void,
    media_feature_callback: unsafe extern "C" fn(ctx: *mut c_void, media_feature: *const CssMediaFeature),
    media_feature_value_callback: unsafe extern "C" fn(
        ctx: *mut c_void,
        media_feature_value: *const CssMediaFeatureValue,
    ),
) {
    unsafe {
        abort_on_panic(|| {
            let Some(input) = bytes_from_raw(input, input_len) else {
                return;
            };

            css_parser::parse_a_media_feature(
                input,
                |media_feature| {
                    media_feature_callback(ctx, &raw const media_feature);
                },
                |media_feature_value| {
                    media_feature_value_callback(ctx, &raw const media_feature_value);
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
