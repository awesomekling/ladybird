// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

//! https://html.spec.whatwg.org/multipage/parsing.html#the-stack-of-open-elements
//!
//! The stack of open elements is said to have an element in a specific scope
//! when it has that element in scope using a list of element types.

use crate::dom_bridge::{DomHandle, DomNamespace};
use crate::tag_names::tags;

/// An entry in the stack of open elements.
/// We cache the tag name and namespace to avoid FFI calls during scope checks.
#[derive(Clone, Debug)]
pub struct StackEntry {
    pub handle: DomHandle,
    pub tag_name: String,
    pub namespace: DomNamespace,
    /// Cached "encoding" attribute for MathML annotation-xml (HTML integration point check).
    pub encoding_attr: Option<String>,
}

impl StackEntry {
    pub fn new(handle: DomHandle, tag_name: String, namespace: DomNamespace) -> Self {
        StackEntry {
            handle,
            tag_name,
            namespace,
            encoding_attr: None,
        }
    }

    pub fn new_with_encoding(
        handle: DomHandle,
        tag_name: String,
        namespace: DomNamespace,
        encoding_attr: Option<String>,
    ) -> Self {
        StackEntry {
            handle,
            tag_name,
            namespace,
            encoding_attr,
        }
    }

    pub fn is_html_element(&self, tag: &str) -> bool {
        self.namespace == DomNamespace::HTML && self.tag_name == tag
    }
}

pub struct StackOfOpenElements {
    elements: Vec<StackEntry>,
}

impl StackOfOpenElements {
    pub fn new() -> Self {
        StackOfOpenElements {
            elements: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: StackEntry) {
        self.elements.push(entry);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#stack-of-open-elements
    /// Pop the current node off the stack, notifying the C++ side.
    pub fn pop(&mut self) -> Option<StackEntry> {
        let entry = self.elements.pop();
        if let Some(ref e) = entry {
            // Notify the C++ side that an element was popped.
            // This is needed for option elements (maybe_clone_into_selectedcontent).
            e.handle.element_popped();
        }
        entry
    }

    /// Clear the stack without callbacks. Used at end of parsing.
    pub fn clear(&mut self) {
        self.elements.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// The current node is the bottommost node in this stack of open elements.
    /// https://html.spec.whatwg.org/multipage/parsing.html#current-node
    pub fn current_node(&self) -> Option<&StackEntry> {
        self.elements.last()
    }

    pub fn current_node_mut(&mut self) -> Option<&mut StackEntry> {
        self.elements.last_mut()
    }

    /// The first element in the stack (the html element).
    pub fn first(&self) -> Option<&StackEntry> {
        self.elements.first()
    }

    pub fn get(&self, index: usize) -> Option<&StackEntry> {
        self.elements.get(index)
    }

    pub fn elements(&self) -> &[StackEntry] {
        &self.elements
    }

    pub fn elements_mut(&mut self) -> &mut Vec<StackEntry> {
        &mut self.elements
    }

    pub fn contains(&self, handle: DomHandle) -> bool {
        self.elements.iter().any(|e| e.handle == handle)
    }

    pub fn contains_tag(&self, tag: &str) -> bool {
        self.elements
            .iter()
            .any(|e| e.namespace == DomNamespace::HTML && e.tag_name == tag)
    }

    pub fn contains_template_element(&self) -> bool {
        self.contains_tag(tags::TEMPLATE)
    }

    /// Pop elements until an element with the given tag name has been popped.
    pub fn pop_until_tag_name_popped(&mut self, tag: &str) {
        while let Some(entry) = self.elements.last() {
            let matches = entry.namespace == DomNamespace::HTML && entry.tag_name == tag;
            // Use self.pop() to call the element_popped callback, but only for option elements
            // which need maybe_clone_into_selectedcontent. For other elements, directly pop
            // to avoid potential side effects.
            self.pop();
            if matches {
                break;
            }
        }
    }

    /// Pop elements until one of the given tag names has been popped.
    pub fn pop_until_one_of_tag_names_popped(&mut self, tags: &[&str]) {
        while let Some(entry) = self.elements.last() {
            let matches =
                entry.namespace == DomNamespace::HTML && tags.contains(&entry.tag_name.as_str());
            self.pop();
            if matches {
                break;
            }
        }
    }

    /// Remove a specific element from the stack (by handle).
    pub fn remove(&mut self, handle: DomHandle) {
        self.elements.retain(|e| e.handle != handle);
    }

    /// Replace an element in the stack.
    pub fn replace(&mut self, old_handle: DomHandle, new_entry: StackEntry) {
        if let Some(pos) = self.elements.iter().position(|e| e.handle == old_handle) {
            self.elements[pos] = new_entry;
        }
    }

    /// Insert an entry immediately below the given target.
    pub fn insert_immediately_below(&mut self, entry: StackEntry, target: DomHandle) {
        if let Some(pos) = self.elements.iter().position(|e| e.handle == target) {
            self.elements.insert(pos + 1, entry);
        }
    }

    /// Find the last element with the given tag name, returning its index.
    pub fn last_element_with_tag_name(&self, tag: &str) -> Option<(usize, &StackEntry)> {
        self.elements
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| e.namespace == DomNamespace::HTML && e.tag_name == tag)
    }

    /// Get the element immediately above (closer to top) the target.
    /// Return the element immediately above (closer to the root) the target in the stack.
    /// In the spec, "above" means toward the root of the tree (lower index in our array).
    pub fn element_immediately_above(&self, target: DomHandle) -> Option<&StackEntry> {
        if let Some(pos) = self.elements.iter().position(|e| e.handle == target) {
            if pos > 0 {
                return Some(&self.elements[pos - 1]);
            }
        }
        None
    }

    /// Get the entry at a given index.
    pub fn entry_at(&self, index: usize) -> &StackEntry {
        &self.elements[index]
    }

    /// Find the index of an element by handle.
    pub fn index_of(&self, handle: DomHandle) -> Option<usize> {
        self.elements.iter().position(|e| e.handle == handle)
    }

    // -----------------------------------------------------------------------
    // Scope checking
    // https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-scope
    // -----------------------------------------------------------------------

    /// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-the-specific-scope
    fn has_in_specific_scope(&self, target_tag: &str, scope_markers: &ScopeMarkers) -> bool {
        for entry in self.elements.iter().rev() {
            if entry.namespace == DomNamespace::HTML && entry.tag_name == target_tag {
                return true;
            }
            if is_scope_marker(entry, scope_markers) {
                return false;
            }
        }
        false
    }

    fn has_element_in_specific_scope(&self, target: DomHandle, scope_markers: &ScopeMarkers) -> bool {
        for entry in self.elements.iter().rev() {
            if entry.handle == target {
                return true;
            }
            if is_scope_marker(entry, scope_markers) {
                return false;
            }
        }
        false
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-scope
    pub fn has_in_scope(&self, tag: &str) -> bool {
        self.has_in_specific_scope(tag, &ScopeMarkers::Default)
    }

    pub fn has_element_in_scope(&self, handle: DomHandle) -> bool {
        self.has_element_in_specific_scope(handle, &ScopeMarkers::Default)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-list-item-scope
    pub fn has_in_list_item_scope(&self, tag: &str) -> bool {
        self.has_in_specific_scope(tag, &ScopeMarkers::ListItem)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-button-scope
    pub fn has_in_button_scope(&self, tag: &str) -> bool {
        self.has_in_specific_scope(tag, &ScopeMarkers::Button)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-table-scope
    pub fn has_in_table_scope(&self, tag: &str) -> bool {
        self.has_in_specific_scope(tag, &ScopeMarkers::Table)
    }

    // -----------------------------------------------------------------------
    // Implied end tags
    // https://html.spec.whatwg.org/multipage/parsing.html#generate-implied-end-tags
    // -----------------------------------------------------------------------

    /// https://html.spec.whatwg.org/multipage/parsing.html#generate-implied-end-tags
    pub fn generate_implied_end_tags(&mut self, exception: Option<&str>) {
        self.pop_while_html_tag_matches(|tag| is_implied_end_tag(tag), exception);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#generate-all-implied-end-tags-thoroughly
    pub fn generate_all_implied_end_tags_thoroughly(&mut self) {
        self.pop_while_html_tag_matches(|tag| is_thorough_implied_end_tag(tag), None);
    }

    /// Pop HTML elements off the stack while the predicate matches the current node's tag name.
    /// Optionally skip popping if the tag matches the exception.
    fn pop_while_html_tag_matches(
        &mut self,
        predicate: impl Fn(&str) -> bool,
        exception: Option<&str>,
    ) {
        loop {
            let should_pop = match self.elements.last() {
                Some(entry) if entry.namespace == DomNamespace::HTML => {
                    if exception.is_some_and(|exc| entry.tag_name == exc) {
                        false
                    } else {
                        predicate(&entry.tag_name)
                    }
                }
                _ => false,
            };
            if should_pop {
                self.pop();
            } else {
                break;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Clear stack back to context
    // https://html.spec.whatwg.org/multipage/parsing.html#clear-the-stack-back-to-a-table-context
    // -----------------------------------------------------------------------

    /// https://html.spec.whatwg.org/multipage/parsing.html#clear-the-stack-back-to-a-table-context
    pub fn clear_back_to_table_context(&mut self) {
        self.clear_back_to_context(&["table", "template", "html"]);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#clear-the-stack-back-to-a-table-body-context
    pub fn clear_back_to_table_body_context(&mut self) {
        self.clear_back_to_context(&["tbody", "tfoot", "thead", "template", "html"]);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#clear-the-stack-back-to-a-table-row-context
    pub fn clear_back_to_table_row_context(&mut self) {
        self.clear_back_to_context(&["tr", "template", "html"]);
    }

    /// Pop elements until the current node is an HTML element with one of the given tag names.
    fn clear_back_to_context(&mut self, context_tags: &[&str]) {
        while let Some(entry) = self.elements.last() {
            if entry.namespace == DomNamespace::HTML
                && context_tags.contains(&entry.tag_name.as_str())
            {
                break;
            }
            self.pop();
        }
    }

    // -----------------------------------------------------------------------
    // Special tag classification
    // https://html.spec.whatwg.org/multipage/parsing.html#special
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // GC integration
    // -----------------------------------------------------------------------

    /// Call the visitor for each DOM handle in the stack.
    pub fn visit_dom_handles(&self, visitor: *mut std::ffi::c_void) {
        for entry in &self.elements {
            crate::dom_bridge::DomHandle::visit_node(visitor, entry.handle);
        }
    }
}

// -----------------------------------------------------------------------
// Scope marker definitions
// -----------------------------------------------------------------------

enum ScopeMarkers {
    Default,
    ListItem,
    Button,
    Table,
}

/// https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-scope
/// The base set of HTML scope markers shared by default, list-item, and button scopes.
fn is_base_html_scope_marker(tag: &str) -> bool {
    matches!(
        tag,
        "applet" | "caption" | "html" | "table" | "td" | "th"
            | "marquee" | "object" | "select" | "template"
    )
}

fn is_scope_marker(entry: &StackEntry, markers: &ScopeMarkers) -> bool {
    match entry.namespace {
        DomNamespace::HTML => match markers {
            ScopeMarkers::Default => is_base_html_scope_marker(&entry.tag_name),
            ScopeMarkers::ListItem => {
                is_base_html_scope_marker(&entry.tag_name)
                    || matches!(entry.tag_name.as_str(), "ol" | "ul")
            }
            ScopeMarkers::Button => {
                is_base_html_scope_marker(&entry.tag_name)
                    || entry.tag_name == "button"
            }
            // https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-table-scope
            // NOTE: Table scope only has HTML element markers, no MathML/SVG markers.
            ScopeMarkers::Table => matches!(
                entry.tag_name.as_str(),
                "html" | "table" | "template"
            ),
        },
        // MathML and SVG scope markers only apply to non-table scope types.
        DomNamespace::MathML if !matches!(markers, ScopeMarkers::Table) => matches!(
            entry.tag_name.as_str(),
            "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
        ),
        DomNamespace::SVG if !matches!(markers, ScopeMarkers::Table) => matches!(
            entry.tag_name.as_str(),
            "foreignObject" | "desc" | "title"
        ),
        _ => false,
    }
}

// -----------------------------------------------------------------------
// Implied end tag sets
// -----------------------------------------------------------------------

/// https://html.spec.whatwg.org/multipage/parsing.html#generate-implied-end-tags
fn is_implied_end_tag(tag: &str) -> bool {
    matches!(
        tag,
        "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
    )
}

/// https://html.spec.whatwg.org/multipage/parsing.html#generate-all-implied-end-tags-thoroughly
fn is_thorough_implied_end_tag(tag: &str) -> bool {
    is_implied_end_tag(tag)
        || matches!(
            tag,
            "caption" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
        )
}

/// https://html.spec.whatwg.org/multipage/parsing.html#special
pub fn is_special_tag(tag: &str, namespace: DomNamespace) -> bool {
    match namespace {
        DomNamespace::HTML => matches!(
            tag,
            "address"
                | "applet"
                | "area"
                | "article"
                | "aside"
                | "base"
                | "basefont"
                | "bgsound"
                | "blockquote"
                | "body"
                | "br"
                | "button"
                | "caption"
                | "center"
                | "col"
                | "colgroup"
                | "dd"
                | "details"
                | "dir"
                | "div"
                | "dl"
                | "dt"
                | "embed"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "frame"
                | "frameset"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "head"
                | "header"
                | "hgroup"
                | "hr"
                | "html"
                | "iframe"
                | "img"
                | "input"
                | "keygen"
                | "li"
                | "link"
                | "listing"
                | "main"
                | "marquee"
                | "menu"
                | "meta"
                | "nav"
                | "noembed"
                | "noframes"
                | "noscript"
                | "object"
                | "ol"
                | "p"
                | "param"
                | "plaintext"
                | "pre"
                | "script"
                | "search"
                | "section"
                | "select"
                | "source"
                | "style"
                | "summary"
                | "table"
                | "tbody"
                | "td"
                | "template"
                | "textarea"
                | "tfoot"
                | "th"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
                | "xmp"
        ),
        DomNamespace::SVG => matches!(tag, "foreignObject" | "desc" | "title"),
        DomNamespace::MathML => {
            matches!(tag, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml")
        }
        _ => false,
    }
}
