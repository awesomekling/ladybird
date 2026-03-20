// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

//! FFI bridge to C++ DOM operations.
//!
//! These extern "C" functions are implemented in C++ (HTMLParserBridge.cpp)
//! and called by the Rust HTML parser to manipulate the DOM tree.

use std::ffi::c_void;

/// Opaque handle to a C++ DOM node (DOM::Node*, DOM::Element*, DOM::Document*, etc).
/// The Rust parser never dereferences these — they are passed back to C++ via bridge functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DomHandle(pub *mut c_void);

unsafe impl Send for DomHandle {}
unsafe impl Sync for DomHandle {}

impl DomHandle {
    pub fn null() -> Self {
        DomHandle(std::ptr::null_mut())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub fn as_ptr(self) -> *mut c_void {
        self.0
    }
}

/// Namespace identifiers matching C++ Web::Namespace constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DomNamespace {
    HTML = 0,
    SVG = 1,
    MathML = 2,
    XLink = 3,
    XML = 4,
    XMLNS = 5,
}

/// Quirks mode values matching C++ DOM::Document::QuirksMode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum QuirksMode {
    No = 0,
    Limited = 1,
    Yes = 2,
}

/// Attribute pair for passing to C++ element creation.
#[repr(C)]
pub struct BridgeAttribute {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
    pub namespace: u8, // DomNamespace value, or 0xFF for no namespace
    pub prefix_ptr: *const u8,
    pub prefix_len: usize,
}

unsafe extern "C" {
    // -----------------------------------------------------------------------
    // Element creation
    // -----------------------------------------------------------------------

    /// Create a DOM element with the given local name, namespace, and optional `is` value.
    /// Returns an opaque handle to the created element.
    pub fn html_parser_bridge_create_element(
        document: *mut c_void,
        local_name: *const u8,
        local_name_len: usize,
        namespace: u8,
        is_value: *const u8,
        is_value_len: usize,
    ) -> *mut c_void;

    /// Append an attribute to an element (only if not already present — first one wins).
    pub fn html_parser_bridge_append_attribute(
        element: *mut c_void,
        name: *const u8,
        name_len: usize,
        value: *const u8,
        value_len: usize,
    );

    /// Set an attribute on an element (overwrites existing).
    pub fn html_parser_bridge_set_attribute(
        element: *mut c_void,
        name: *const u8,
        name_len: usize,
        value: *const u8,
        value_len: usize,
    );

    /// Set a namespaced attribute on an element.
    pub fn html_parser_bridge_set_attribute_ns(
        element: *mut c_void,
        namespace: u8,
        prefix: *const u8,
        prefix_len: usize,
        local_name: *const u8,
        local_name_len: usize,
        value: *const u8,
        value_len: usize,
    );

    // -----------------------------------------------------------------------
    // Text and Comment creation
    // -----------------------------------------------------------------------

    /// Create a text node. Data is UTF-8 encoded.
    pub fn html_parser_bridge_create_text_node(
        document: *mut c_void,
        data: *const u8,
        data_len: usize,
    ) -> *mut c_void;

    /// Create a comment node. Data is UTF-8 encoded.
    pub fn html_parser_bridge_create_comment(
        document: *mut c_void,
        data: *const u8,
        data_len: usize,
    ) -> *mut c_void;

    /// Append UTF-8 text data to an existing text node.
    pub fn html_parser_bridge_append_text(
        text_node: *mut c_void,
        data: *const u8,
        data_len: usize,
    );

    // -----------------------------------------------------------------------
    // Document type
    // -----------------------------------------------------------------------

    /// Create and insert a DocumentType node.
    pub fn html_parser_bridge_insert_doctype(
        document: *mut c_void,
        name: *const u8,
        name_len: usize,
        public_id: *const u8,
        public_id_len: usize,
        system_id: *const u8,
        system_id_len: usize,
    );

    /// Set the document's quirks mode.
    pub fn html_parser_bridge_set_quirks_mode(document: *mut c_void, mode: u8);

    // -----------------------------------------------------------------------
    // Tree manipulation
    // -----------------------------------------------------------------------

    /// Insert a node as a child of parent, before the given sibling.
    /// If before_sibling is null, append as last child.
    pub fn html_parser_bridge_insert_before(
        parent: *mut c_void,
        node: *mut c_void,
        before_sibling: *mut c_void,
    );

    /// Remove a node from its parent.
    pub fn html_parser_bridge_remove_node(node: *mut c_void);

    // -----------------------------------------------------------------------
    // Node queries
    // -----------------------------------------------------------------------

    /// Get the parent node. Returns null if none.
    pub fn html_parser_bridge_parent(node: *mut c_void) -> *mut c_void;

    /// Get the last child. Returns null if none.
    pub fn html_parser_bridge_last_child(node: *mut c_void) -> *mut c_void;

    /// Get the next sibling. Returns null if none.
    pub fn html_parser_bridge_next_sibling(node: *mut c_void) -> *mut c_void;

    /// Check if a node is a Text node.
    pub fn html_parser_bridge_is_text_node(node: *mut c_void) -> bool;

    /// Check if a node is an Element.
    pub fn html_parser_bridge_is_element(node: *mut c_void) -> bool;

    // -----------------------------------------------------------------------
    // Element queries
    // -----------------------------------------------------------------------

    /// Get the local name of an element as a UTF-8 string.
    /// Writes to out_ptr and out_len. The string is valid until the next bridge call.
    pub fn html_parser_bridge_element_local_name(
        element: *mut c_void,
        out_ptr: *mut *const u8,
        out_len: *mut usize,
    );

    /// Get the namespace of an element as a DomNamespace value.
    pub fn html_parser_bridge_element_namespace(element: *mut c_void) -> u8;

    // -----------------------------------------------------------------------
    // Template
    // -----------------------------------------------------------------------

    /// Get the template element's content document fragment.
    pub fn html_parser_bridge_template_contents(element: *mut c_void) -> *mut c_void;

    // -----------------------------------------------------------------------
    // Document queries
    // -----------------------------------------------------------------------

    /// Get the document node handle (as a DOM::Node*).
    pub fn html_parser_bridge_document_node(document: *mut c_void) -> *mut c_void;

    // -----------------------------------------------------------------------
    // Form association
    // -----------------------------------------------------------------------

    /// Associate an element with a form element.
    pub fn html_parser_bridge_associate_with_form(
        element: *mut c_void,
        form: *mut c_void,
    );

    // -----------------------------------------------------------------------
    // Script execution
    // -----------------------------------------------------------------------

    /// Prepare and potentially execute a script element.
    /// For external scripts, blocks until loaded and executed.
    /// Returns true if the parser should stop.
    pub fn html_parser_bridge_execute_script(element: *mut c_void, document: *mut c_void, script_nesting_level: usize) -> bool;

    // -----------------------------------------------------------------------
    // GC integration
    // -----------------------------------------------------------------------

    /// Post-creation setup for elements (media muted attribute, etc).
    pub fn html_parser_bridge_post_create_element(element: *mut c_void);

    /// Notify that an element was popped from the stack of open elements.
    pub fn html_parser_bridge_element_popped(element: *mut c_void);

    /// Set parser_document and force_async=false on a script element.
    pub fn html_parser_bridge_setup_script_element(
        element: *mut c_void,
        document: *mut c_void,
    );

    /// Move all children of `from` node to `to` node.
    pub fn html_parser_bridge_reparent_children(from: *mut c_void, to: *mut c_void);

    /// Handle declarative shadow DOM template.
    pub fn html_parser_bridge_handle_declarative_shadow_template(
        template_element: *mut c_void,
        host_element: *mut c_void,
        document: *mut c_void,
        shadowrootmode: *const u8,
        shadowrootmode_len: usize,
        shadowrootclonable: bool,
        shadowrootserializable: bool,
        shadowrootdelegatesfocus: bool,
    ) -> bool;

    /// Process pending parsing-blocking scripts after script nesting level reaches 0.
    pub fn html_parser_bridge_process_pending_scripts(document: *mut c_void);

    /// Visit a DOM handle for garbage collection.
    /// The visitor_ctx is an opaque pointer to a C++ GC::Cell::Visitor.
    pub fn html_parser_bridge_visit_node(
        visitor_ctx: *mut c_void,
        node: *mut c_void,
    );
}

// -----------------------------------------------------------------------
// Safe wrappers
// -----------------------------------------------------------------------

impl DomHandle {
    pub fn create_element(
        document: DomHandle,
        local_name: &str,
        namespace: DomNamespace,
        is_value: Option<&str>,
    ) -> DomHandle {
        let (is_ptr, is_len) = match is_value {
            Some(v) => (v.as_ptr(), v.len()),
            None => (std::ptr::null(), 0),
        };
        DomHandle(unsafe {
            html_parser_bridge_create_element(
                document.as_ptr(),
                local_name.as_ptr(),
                local_name.len(),
                namespace as u8,
                is_ptr,
                is_len,
            )
        })
    }

    /// Append an attribute (first one wins — skips if already present).
    pub fn append_attribute(self, name: &str, value: &str) {
        unsafe {
            html_parser_bridge_append_attribute(
                self.as_ptr(),
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            );
        }
    }

    pub fn set_attribute(self, name: &str, value: &str) {
        unsafe {
            html_parser_bridge_set_attribute(
                self.as_ptr(),
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            );
        }
    }

    pub fn set_attribute_ns(
        self,
        namespace: DomNamespace,
        prefix: &str,
        local_name: &str,
        value: &str,
    ) {
        unsafe {
            html_parser_bridge_set_attribute_ns(
                self.as_ptr(),
                namespace as u8,
                prefix.as_ptr(),
                prefix.len(),
                local_name.as_ptr(),
                local_name.len(),
                value.as_ptr(),
                value.len(),
            );
        }
    }

    pub fn create_text_node(document: DomHandle, data: &str) -> DomHandle {
        DomHandle(unsafe {
            html_parser_bridge_create_text_node(
                document.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        })
    }

    pub fn create_comment(document: DomHandle, data: &str) -> DomHandle {
        DomHandle(unsafe {
            html_parser_bridge_create_comment(
                document.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        })
    }

    pub fn append_text(self, data: &str) {
        unsafe {
            html_parser_bridge_append_text(
                self.as_ptr(),
                data.as_ptr(),
                data.len(),
            );
        }
    }

    pub fn insert_doctype(
        document: DomHandle,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) {
        unsafe {
            html_parser_bridge_insert_doctype(
                document.as_ptr(),
                name.as_ptr(),
                name.len(),
                public_id.as_ptr(),
                public_id.len(),
                system_id.as_ptr(),
                system_id.len(),
            );
        }
    }

    pub fn set_quirks_mode(document: DomHandle, mode: QuirksMode) {
        unsafe {
            html_parser_bridge_set_quirks_mode(document.as_ptr(), mode as u8);
        }
    }

    pub fn insert_before(parent: DomHandle, node: DomHandle, before_sibling: Option<DomHandle>) {
        unsafe {
            html_parser_bridge_insert_before(
                parent.as_ptr(),
                node.as_ptr(),
                before_sibling.map_or(std::ptr::null_mut(), |h| h.as_ptr()),
            );
        }
    }

    pub fn remove(self) {
        unsafe {
            html_parser_bridge_remove_node(self.as_ptr());
        }
    }

    pub fn parent(self) -> Option<DomHandle> {
        let p = DomHandle(unsafe { html_parser_bridge_parent(self.as_ptr()) });
        if p.is_null() { None } else { Some(p) }
    }

    pub fn last_child(self) -> Option<DomHandle> {
        let c = DomHandle(unsafe { html_parser_bridge_last_child(self.as_ptr()) });
        if c.is_null() { None } else { Some(c) }
    }

    pub fn next_sibling(self) -> Option<DomHandle> {
        let s = DomHandle(unsafe { html_parser_bridge_next_sibling(self.as_ptr()) });
        if s.is_null() { None } else { Some(s) }
    }

    pub fn is_text_node(self) -> bool {
        unsafe { html_parser_bridge_is_text_node(self.as_ptr()) }
    }

    pub fn is_element(self) -> bool {
        unsafe { html_parser_bridge_is_element(self.as_ptr()) }
    }

    pub fn template_contents(self) -> DomHandle {
        DomHandle(unsafe { html_parser_bridge_template_contents(self.as_ptr()) })
    }

    pub fn document_node(document: DomHandle) -> DomHandle {
        DomHandle(unsafe { html_parser_bridge_document_node(document.as_ptr()) })
    }

    pub fn associate_with_form(self, form: DomHandle) {
        unsafe {
            html_parser_bridge_associate_with_form(self.as_ptr(), form.as_ptr());
        }
    }

    pub fn execute_script(self, document: DomHandle, script_nesting_level: usize) -> bool {
        unsafe { html_parser_bridge_execute_script(self.as_ptr(), document.as_ptr(), script_nesting_level) }
    }

    pub fn setup_script_element(self, document: DomHandle) {
        unsafe {
            html_parser_bridge_setup_script_element(self.as_ptr(), document.as_ptr());
        }
    }
}
