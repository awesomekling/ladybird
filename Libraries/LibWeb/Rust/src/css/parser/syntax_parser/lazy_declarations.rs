/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The declarations of a parsed style rule, which a sheet keeps as the text of the rule's block until something reads
//! them. Most of a large sheet's style rules match no element, and their declarations are never read.

use std::ops::Range;
use std::ptr::NonNull;
use std::sync::{Arc, OnceLock};

use crate::css::css_tokenizer::TokenizerInput;
use crate::css::declaration_block::DeclarationBlockData;
use crate::css::parser::value_parser::ParseContext;

/// What of a sheet's parse context parsing one of its blocks later reads, owned by the blocks.
pub(crate) struct LazyDeclarationContext {
    in_quirks_mode: bool,
    is_svg_presentation_attribute: bool,
    is_ua_style_sheet: bool,
    document_url: Box<[u8]>,
    document_base_url: Box<[u8]>,
}

impl LazyDeclarationContext {
    /// The parts of `context` a block parsed later reads, where it reads nothing it cannot own: no enclosing value
    /// context or declared namespace.
    pub(crate) fn capture(context: Option<&ParseContext>) -> Option<Self> {
        let Some(context) = context else {
            return Some(Self {
                in_quirks_mode: false,
                is_svg_presentation_attribute: false,
                is_ua_style_sheet: false,
                document_url: Box::default(),
                document_base_url: Box::default(),
            });
        };
        if context.value_context_count != 0 || !context.declared_namespaces.is_null() {
            return None;
        }
        let bytes = |pointer: *const u8, length: usize| -> Box<[u8]> {
            if pointer.is_null() || length == 0 {
                return Box::default();
            }
            // SAFETY: The context's URLs are readable for their lengths while the sheet parses.
            unsafe { std::slice::from_raw_parts(pointer, length) }.into()
        };
        Some(Self {
            in_quirks_mode: context.in_quirks_mode,
            is_svg_presentation_attribute: context.is_svg_presentation_attribute,
            is_ua_style_sheet: context.is_ua_style_sheet,
            document_url: bytes(context.document_url, context.document_url_length),
            document_base_url: bytes(context.document_base_url, context.document_base_url_length),
        })
    }

    fn parse_context(&self) -> ParseContext {
        ParseContext {
            in_quirks_mode: self.in_quirks_mode,
            is_svg_presentation_attribute: self.is_svg_presentation_attribute,
            is_ua_style_sheet: self.is_ua_style_sheet,
            document_url: self.document_url.as_ptr(),
            document_url_length: self.document_url.len(),
            document_base_url: self.document_base_url.as_ptr(),
            document_base_url_length: self.document_base_url.len(),
            ..ParseContext::default()
        }
    }
}

/// The text of a sheet's lazy blocks, each once, in ASCII where a block is ASCII.
#[derive(Default)]
pub(crate) struct LazyDeclarationText {
    pub(crate) ascii: Box<[u8]>,
    pub(crate) utf16: Box<[u16]>,
}

/// The source of a sheet's lazy blocks: their text, which the sheet's parse sets as it finishes, and the sheet's parse
/// context.
pub(crate) struct LazyDeclarationSource {
    pub(crate) text: OnceLock<LazyDeclarationText>,
    pub(crate) context: LazyDeclarationContext,
}

/// A style rule's block, parsed into declarations the first time they are read.
pub(crate) struct LazyDeclarations {
    source: Arc<LazyDeclarationSource>,
    range: Range<u32>,
    is_ascii: bool,
    parsed: OnceLock<Arc<DeclarationBlockData>>,
}

impl LazyDeclarations {
    pub(crate) fn new(source: Arc<LazyDeclarationSource>, range: Range<usize>, is_ascii: bool) -> Self {
        let range = u32::try_from(range.start).expect("a style sheet cannot have 4G code units")
            ..u32::try_from(range.end).expect("a style sheet cannot have 4G code units");
        Self {
            source,
            range,
            is_ascii,
            parsed: OnceLock::new(),
        }
    }

    pub(crate) fn range(&self) -> (Range<usize>, bool) {
        (self.range.start as usize..self.range.end as usize, self.is_ascii)
    }

    /// The block's text, between its braces.
    pub(crate) fn text(&self) -> TokenizerInput<'_> {
        let text = self
            .source
            .text
            .get()
            .expect("a lazy block is read after its sheet's parse");
        let range = self.range.start as usize..self.range.end as usize;
        if self.is_ascii {
            TokenizerInput::Ascii(&text.ascii[range])
        } else {
            TokenizerInput::Utf16(&text.utf16[range])
        }
    }

    pub(crate) fn data(&self) -> &Arc<DeclarationBlockData> {
        self.parsed.get_or_init(|| {
            let context = self.source.context.parse_context();
            super::parse_style_rule_block(self.text(), &context)
        })
    }
}

/// The declarations of a parsed rule: parsed, or a block parsed the first time they are read, in one tagged word.
pub(crate) struct RuleDeclarations(NonNull<u8>);

// SAFETY: Both kinds of declarations are immutable behind an Arc, which is Send and Sync.
unsafe impl Send for RuleDeclarations {}
unsafe impl Sync for RuleDeclarations {}

const LAZY_TAG: usize = 1;

const _: () = assert!(align_of::<DeclarationBlockData>() > LAZY_TAG && align_of::<LazyDeclarations>() > LAZY_TAG);

impl RuleDeclarations {
    pub(crate) fn parsed(data: Arc<DeclarationBlockData>) -> Self {
        Self(NonNull::new(Arc::into_raw(data).cast_mut().cast()).unwrap())
    }

    pub(crate) fn lazy(lazy: Arc<LazyDeclarations>) -> Self {
        Self(
            NonNull::new(
                Arc::into_raw(lazy)
                    .cast_mut()
                    .cast::<u8>()
                    .map_addr(|address| address | LAZY_TAG),
            )
            .unwrap(),
        )
    }

    fn lazy_pointer(&self) -> Option<*const LazyDeclarations> {
        let address = self.0.as_ptr();
        (address.addr() & LAZY_TAG != 0).then(|| address.map_addr(|address| address & !LAZY_TAG).cast_const().cast())
    }

    /// The block, unless it is still to be parsed.
    pub(crate) fn as_lazy(&self) -> Option<&LazyDeclarations> {
        // SAFETY: A tagged word holds one strong reference to its lazy declarations.
        self.lazy_pointer().map(|pointer| unsafe { &*pointer })
    }

    /// The declarations, parsed now where they were still to be parsed.
    pub(crate) fn data(&self) -> Arc<DeclarationBlockData> {
        match self.as_lazy() {
            Some(lazy) => lazy.data().clone(),
            None => {
                let pointer = self.0.as_ptr().cast_const().cast::<DeclarationBlockData>();
                // SAFETY: An untagged word holds one strong reference to its declarations.
                unsafe {
                    Arc::increment_strong_count(pointer);
                    Arc::from_raw(pointer)
                }
            }
        }
    }

    /// What identifies the declarations, which stays the same as they are parsed.
    pub(crate) fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }

    /// Whether the declarations may define transitions. A block still to be parsed may where its text names one.
    pub(crate) fn may_declare_transitions(&self) -> bool {
        match self.as_lazy() {
            Some(lazy) if lazy.parsed.get().is_none() => contains_ascii_case_insensitive(lazy.text(), b"transition"),
            _ => crate::css::style::bridge::declares_transitions(&self.data().properties),
        }
    }
}

impl Clone for RuleDeclarations {
    fn clone(&self) -> Self {
        // SAFETY: The word holds one strong reference of the kind its tag names.
        unsafe {
            match self.lazy_pointer() {
                Some(pointer) => Arc::increment_strong_count(pointer),
                None => Arc::increment_strong_count(self.0.as_ptr().cast_const().cast::<DeclarationBlockData>()),
            }
        }
        Self(self.0)
    }
}

impl Drop for RuleDeclarations {
    fn drop(&mut self) {
        // SAFETY: The word holds one strong reference of the kind its tag names, which this releases.
        unsafe {
            match self.lazy_pointer() {
                Some(pointer) => drop(Arc::from_raw(pointer)),
                None => drop(Arc::from_raw(
                    self.0.as_ptr().cast_const().cast::<DeclarationBlockData>(),
                )),
            }
        }
    }
}

/// Whether `text` holds `needle`, compared ASCII case-insensitively.
pub(crate) fn contains_ascii_case_insensitive(text: TokenizerInput<'_>, needle: &[u8]) -> bool {
    fn contains<T: Copy>(text: &[T], needle: &[u8], unit: impl Fn(T) -> u16) -> bool {
        text.windows(needle.len()).any(|window| {
            window.iter().zip(needle).all(|(&unit_of_text, &expected)| {
                let value = unit(unit_of_text);
                value < 128 && (value as u8).eq_ignore_ascii_case(&expected)
            })
        })
    }
    match text {
        TokenizerInput::Ascii(units) => contains(units, needle, u16::from),
        TokenizerInput::Utf16(units) => contains(units, needle, |unit| unit),
    }
}
