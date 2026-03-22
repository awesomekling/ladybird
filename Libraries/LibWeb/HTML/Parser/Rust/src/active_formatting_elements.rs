// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

//! https://html.spec.whatwg.org/multipage/parsing.html#the-list-of-active-formatting-elements
//!
//! The list of active formatting elements is used to handle mis-nested formatting element tags.

use crate::dom_bridge::{DomHandle, DomNamespace};
use crate::token::Token;

/// An entry in the list of active formatting elements.
#[derive(Clone, Debug)]
pub enum FormattingEntry {
    /// A marker entry, inserted when entering applet, object, marquee, template,
    /// td, th, or caption elements.
    Marker,

    /// A formatting element entry, with a copy of the token that created it.
    Element {
        handle: DomHandle,
        tag_name: String,
        namespace: DomNamespace,
        token: Token,
    },
}

impl FormattingEntry {
    pub fn is_marker(&self) -> bool {
        matches!(self, FormattingEntry::Marker)
    }

    pub fn handle(&self) -> Option<DomHandle> {
        match self {
            FormattingEntry::Marker => None,
            FormattingEntry::Element { handle, .. } => Some(*handle),
        }
    }

    pub fn tag_name(&self) -> Option<&str> {
        match self {
            FormattingEntry::Marker => None,
            FormattingEntry::Element { tag_name, .. } => Some(tag_name),
        }
    }

    pub fn token(&self) -> Option<&Token> {
        match self {
            FormattingEntry::Marker => None,
            FormattingEntry::Element { token, .. } => Some(token),
        }
    }
}

pub struct ListOfActiveFormattingElements {
    entries: Vec<FormattingEntry>,
}

impl ListOfActiveFormattingElements {
    pub fn new() -> Self {
        ListOfActiveFormattingElements {
            entries: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[FormattingEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut Vec<FormattingEntry> {
        &mut self.entries
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#push-onto-the-list-of-active-formatting-elements
    /// When the steps below require the UA to push onto the list of active formatting elements
    /// an element element, the UA must perform the following steps:
    pub fn add(&mut self, handle: DomHandle, tag_name: String, namespace: DomNamespace, token: Token) {
        // 1. If there are already three elements in the list of active formatting elements after
        //    the last marker, if any, or anywhere in the list if there is no marker, that have the
        //    same tag name, namespace, and attributes as element, then remove the earliest such
        //    element from the list of active formatting elements.
        //    (Noah's Ark clause)
        let mut count = 0;
        let mut earliest_match_index = None;

        let new_attributes = token.attributes();

        // Walk backward from end to find last marker (or start of list).
        for i in (0..self.entries.len()).rev() {
            if self.entries[i].is_marker() {
                break;
            }
            if let FormattingEntry::Element {
                tag_name: ref entry_tag,
                namespace: ref entry_namespace,
                token: ref entry_token,
                ..
            } = self.entries[i]
            {
                if entry_tag == &tag_name
                    && *entry_namespace == namespace
                    && tokens_have_same_attributes(entry_token, new_attributes)
                {
                    count += 1;
                    earliest_match_index = Some(i);
                }
            }
        }

        if count >= 3 {
            if let Some(idx) = earliest_match_index {
                self.entries.remove(idx);
            }
        }

        // 2. Add element to the list of active formatting elements.
        self.entries.push(FormattingEntry::Element {
            handle,
            tag_name,
            namespace,
            token,
        });
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#push-onto-the-list-of-active-formatting-elements
    pub fn add_marker(&mut self) {
        self.entries.push(FormattingEntry::Marker);
    }

    /// Remove the entry for the given element.
    pub fn remove(&mut self, handle: DomHandle) {
        self.entries.retain(|e| match e {
            FormattingEntry::Marker => true,
            FormattingEntry::Element { handle: h, .. } => *h != handle,
        });
    }

    /// Replace an element entry.
    pub fn replace(
        &mut self,
        old_handle: DomHandle,
        new_handle: DomHandle,
        new_tag_name: String,
        new_namespace: DomNamespace,
        new_token: Token,
    ) {
        if let Some(pos) = self.entries.iter().position(|e| e.handle() == Some(old_handle)) {
            self.entries[pos] = FormattingEntry::Element {
                handle: new_handle,
                tag_name: new_tag_name,
                namespace: new_namespace,
                token: new_token,
            };
        }
    }

    /// Check if the list contains an element with the given handle.
    pub fn contains(&self, handle: DomHandle) -> bool {
        self.entries.iter().any(|e| e.handle() == Some(handle))
    }

    /// Find the last element with the given tag name before the last marker.
    pub fn last_element_with_tag_name_before_marker(&self, tag: &str) -> Option<(usize, DomHandle)> {
        for i in (0..self.entries.len()).rev() {
            if self.entries[i].is_marker() {
                return None;
            }
            if let FormattingEntry::Element {
                tag_name, handle, ..
            } = &self.entries[i]
            {
                if tag_name == tag {
                    return Some((i, *handle));
                }
            }
        }
        None
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#clear-the-list-of-active-formatting-elements-up-to-the-last-marker
    /// Clear the list up to the last marker.
    pub fn clear_up_to_last_marker(&mut self) {
        while let Some(entry) = self.entries.pop() {
            if entry.is_marker() {
                break;
            }
        }
    }

    /// Get the index of an element in the list.
    pub fn index_of(&self, handle: DomHandle) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.handle() == Some(handle))
    }

    /// Insert at a specific index.
    pub fn insert_at(
        &mut self,
        index: usize,
        handle: DomHandle,
        tag_name: String,
        namespace: DomNamespace,
        token: Token,
    ) {
        self.entries.insert(
            index,
            FormattingEntry::Element {
                handle,
                tag_name,
                namespace,
                token,
            },
        );
    }

    /// Visit DOM handles for GC.
    pub fn visit_dom_handles(&self, visitor: *mut std::ffi::c_void) {
        for entry in &self.entries {
            if let FormattingEntry::Element { handle, .. } = entry {
                unsafe {
                    crate::dom_bridge::html_parser_bridge_visit_node(visitor, handle.as_ptr());
                }
            }
        }
    }
}

/// Check if two tokens have the same attributes (for Noah's Ark clause).
fn tokens_have_same_attributes(
    token: &Token,
    new_attributes: &[crate::token::Attribute],
) -> bool {
    let old_attrs = token.attributes();
    if old_attrs.len() != new_attributes.len() {
        return false;
    }
    for old in old_attrs {
        let found = new_attributes
            .iter()
            .any(|new| old.local_name == new.local_name && old.value == new.value);
        if !found {
            return false;
        }
    }
    true
}
