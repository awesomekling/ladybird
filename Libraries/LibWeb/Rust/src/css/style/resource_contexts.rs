/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a `url()` resolves against: the document's base URL, and the resource context of the style
//! sheet a rule came from, keyed by the native sheet's identity (an imported sheet's own, not its
//! importer's). The host lends them at each style transaction boundary; the engine keeps a copy.

use std::collections::HashMap;

use super::bridge::{FfiDocumentStyleComputationInputs, FfiHostHandle, FfiStyleSheetResourceContextEntry};
use crate::css::style_compute::FfiStyleSheetResourceContext;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct StyleSheetResourceContext {
    pub(crate) base_url: Box<[u8]>,
    pub(crate) has_base_url: bool,
    pub(crate) origin_clean: bool,
}

#[derive(Debug, Default)]
pub(crate) struct DocumentResourceContexts {
    pub(crate) document_base_url: Box<[u8]>,
    by_source: HashMap<u64, StyleSheetResourceContext>,
}

impl DocumentResourceContexts {
    /// Copy what the host lent with the inputs, and clear the borrowed fields so the inputs compare
    /// by value from here on.
    ///
    /// # Safety
    /// The borrowed fields must name live buffers of their stated lengths for this call.
    pub(crate) unsafe fn take_from(inputs: &mut FfiDocumentStyleComputationInputs) -> Self {
        let bytes = |address: *const u8, length: usize| -> Box<[u8]> {
            if address.is_null() || length == 0 {
                Box::default()
            } else {
                unsafe { std::slice::from_raw_parts(address, length) }.into()
            }
        };
        let document_base_url = bytes(
            inputs.document_base_url.as_pointer().cast(),
            inputs.document_base_url_length,
        );
        let entries =
            if inputs.style_sheet_resource_contexts.is_none() || inputs.style_sheet_resource_context_count == 0 {
                &[][..]
            } else {
                unsafe {
                    std::slice::from_raw_parts(
                        inputs
                            .style_sheet_resource_contexts
                            .as_pointer()
                            .cast::<FfiStyleSheetResourceContextEntry>(),
                        inputs.style_sheet_resource_context_count,
                    )
                }
            };
        let by_source = entries
            .iter()
            .map(|entry| {
                (
                    entry.source_identity,
                    StyleSheetResourceContext {
                        base_url: bytes(entry.base_url, entry.base_url_length),
                        has_base_url: entry.has_base_url,
                        origin_clean: entry.origin_clean,
                    },
                )
            })
            .collect();
        inputs.document_base_url = FfiHostHandle::default();
        inputs.document_base_url_length = 0;
        inputs.style_sheet_resource_contexts = FfiHostHandle::default();
        inputs.style_sheet_resource_context_count = 0;
        Self {
            document_base_url,
            by_source,
        }
    }

    /// Whether a record resolved against these contexts could resolve differently against `next`.
    /// A sheet that joins or leaves changes no URL another sheet's rules resolved.
    pub(crate) fn moved_for_records(&self, next: &Self) -> bool {
        self.document_base_url != next.document_base_url
            || self
                .by_source
                .iter()
                .any(|(source, context)| next.by_source.get(source).is_some_and(|next| next != context))
    }

    pub(crate) fn for_source(&self, source_identity: u64) -> Option<&StyleSheetResourceContext> {
        self.by_source.get(&source_identity)
    }
}

impl StyleSheetResourceContext {
    /// The context as the drive reads it, borrowing this one's base URL.
    pub(crate) fn as_drive_context(&self) -> FfiStyleSheetResourceContext {
        FfiStyleSheetResourceContext {
            base_url: if self.has_base_url {
                self.base_url.as_ptr()
            } else {
                std::ptr::null()
            },
            base_url_length: if self.has_base_url { self.base_url.len() } else { 0 },
            has_value: true,
            origin_clean: self.origin_clean,
        }
    }
}
