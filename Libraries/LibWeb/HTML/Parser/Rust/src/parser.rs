// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

//! Rust implementation of the HTML parser.
//! https://html.spec.whatwg.org/multipage/parsing.html
//!
//! This is a faithful port of the WHATWG HTML parsing algorithm, with spec text
//! as comments throughout.

use crate::active_formatting_elements::{FormattingEntry, ListOfActiveFormattingElements};
use crate::dom_bridge::{DomHandle, DomNamespace, QuirksMode};
use crate::stack_of_open_elements::{is_special_tag, StackEntry, StackOfOpenElements};
use crate::token::{Token, TokenPayload, TokenType};
use crate::tokenizer::{HtmlTokenizer, State as TokenizerState};

/// https://html.spec.whatwg.org/multipage/parsing.html#insertion-mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertionMode {
    Initial,
    BeforeHTML,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

/// The adjusted insertion location for a node.
/// https://html.spec.whatwg.org/multipage/parsing.html#appropriate-place-for-inserting-a-node
struct AdjustedInsertionLocation {
    parent: DomHandle,
    insert_before_sibling: Option<DomHandle>,
}

/// The HTML parser.
/// https://html.spec.whatwg.org/multipage/parsing.html#the-html-parser
#[allow(dead_code)]
pub struct HtmlParser {
    // The tokenizer.
    pub tokenizer: HtmlTokenizer,

    // https://html.spec.whatwg.org/multipage/parsing.html#insertion-mode
    insertion_mode: InsertionMode,
    original_insertion_mode: InsertionMode,

    // https://html.spec.whatwg.org/multipage/parsing.html#the-stack-of-open-elements
    stack_of_open_elements: StackOfOpenElements,

    // https://html.spec.whatwg.org/multipage/parsing.html#the-stack-of-template-insertion-modes
    stack_of_template_insertion_modes: Vec<InsertionMode>,

    // https://html.spec.whatwg.org/multipage/parsing.html#the-list-of-active-formatting-elements
    list_of_active_formatting_elements: ListOfActiveFormattingElements,

    // DOM handles
    document: DomHandle,
    head_element: Option<DomHandle>,
    form_element: Option<DomHandle>,
    context_element: Option<DomHandle>,

    // https://html.spec.whatwg.org/multipage/parsing.html#pending-table-character-tokens
    pending_table_character_tokens: Vec<Token>,

    // Character insertion buffering (optimization).
    character_insertion_node: Option<DomHandle>,
    character_insertion_builder: String,

    // Parser state flags.
    /// https://html.spec.whatwg.org/multipage/parsing.html#next-line-feed
    next_line_feed_can_be_ignored: bool,
    /// https://html.spec.whatwg.org/multipage/parsing.html#foster-parent
    foster_parenting: bool,
    /// https://html.spec.whatwg.org/multipage/parsing.html#frameset-ok-flag
    frameset_ok: bool,
    /// Whether we are parsing a fragment.
    parsing_fragment: bool,
    /// https://html.spec.whatwg.org/multipage/parsing.html#scripting-flag
    scripting_enabled: bool,
    /// Whether parser was invoked via document.write().
    invoked_via_document_write: bool,
    /// Whether the parser has been aborted.
    aborted: bool,
    /// https://html.spec.whatwg.org/multipage/parsing.html#parser-pause-flag
    parser_pause_flag: bool,
    /// Whether we should stop parsing.
    stop_parsing: bool,
    /// https://html.spec.whatwg.org/multipage/parsing.html#script-nesting-level
    script_nesting_level: usize,

    /// Used to signal reprocessing in the main loop.
    reprocess: bool,
}

impl HtmlParser {
    /// Create a new HTML parser for the given document and input.
    pub fn new(document: DomHandle, input: Vec<u32>, scripting_enabled: bool) -> Self {
        HtmlParser {
            tokenizer: HtmlTokenizer::new(input),
            insertion_mode: InsertionMode::Initial,
            original_insertion_mode: InsertionMode::Initial,
            stack_of_open_elements: StackOfOpenElements::new(),
            stack_of_template_insertion_modes: Vec::new(),
            list_of_active_formatting_elements: ListOfActiveFormattingElements::new(),
            document,
            head_element: None,
            form_element: None,
            context_element: None,
            pending_table_character_tokens: Vec::new(),
            character_insertion_node: None,
            character_insertion_builder: String::new(),
            next_line_feed_can_be_ignored: false,
            foster_parenting: false,
            frameset_ok: true,
            parsing_fragment: false,
            scripting_enabled,
            invoked_via_document_write: false,
            aborted: false,
            parser_pause_flag: false,
            stop_parsing: false,
            script_nesting_level: 0,
            reprocess: false,
        }
    }

    // =======================================================================
    // Main parsing loop
    // https://html.spec.whatwg.org/multipage/parsing.html#tree-construction
    // =======================================================================

    /// Run the parser, stopping at the tokenizer's insertion point.
    /// Used for document.write() re-entrant parsing.
    pub fn run_stop_at_insertion_point(&mut self) {
        self.stop_parsing = false;

        loop {
            let token = match self.tokenizer.next_token(true, false) {
                Some(t) => t,
                None => break,
            };

            if self.next_line_feed_can_be_ignored {
                self.next_line_feed_can_be_ignored = false;
                if token.is_character() && token.code_point == '\n' as u32 {
                    continue;
                }
            }

            self.process_token(&token);

            if token.is_eof() && self.tokenizer.is_eof_inserted() {
                break;
            }

            if self.stop_parsing {
                break;
            }
        }

        self.flush_character_insertions();
    }

    /// Run the parser.
    /// https://html.spec.whatwg.org/multipage/parsing.html#tree-construction
    pub fn run(&mut self) {
        self.stop_parsing = false;

        loop {
            let token = match self.tokenizer.next_token(false, false) {
                Some(t) => t,
                None => break,
            };

            // https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inbody
            // If the next token is a U+000A LINE FEED (LF) character token, then ignore that token
            // and move on to the next one. (Newlines at the start of pre/textarea/listing elements are ignored.)
            if self.next_line_feed_can_be_ignored {
                self.next_line_feed_can_be_ignored = false;
                if token.is_character() && token.code_point == '\n' as u32 {
                    continue;
                }
            }

            self.process_token(&token);

            if token.is_eof() && self.tokenizer.is_eof_inserted() {
                break;
            }

            if self.stop_parsing {
                break;
            }
        }

        self.flush_character_insertions();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#tree-construction-dispatcher
    /// As each token is emitted from the tokenizer, the user agent must follow the appropriate
    /// steps from the following list, known as the tree construction dispatcher:
    fn process_token(&mut self, token: &Token) {
        self.reprocess = true;
        while self.reprocess {
            self.reprocess = false;

            // https://html.spec.whatwg.org/multipage/parsing.html#tree-construction-dispatcher
            if self.stack_of_open_elements.is_empty()
                || self.adjusted_current_node_namespace() == Some(DomNamespace::HTML)
                || (self.adjusted_current_node_is_mathml_text_integration_point()
                    && token.is_start_tag()
                    && token.tag_name() != "mglyph"
                    && token.tag_name() != "malignmark")
                || (self.adjusted_current_node_is_mathml_text_integration_point()
                    && token.is_character())
                || (self.adjusted_current_node_is_mathml_annotation_xml()
                    && token.is_start_tag()
                    && token.tag_name() == "svg")
                || (self.adjusted_current_node_is_html_integration_point()
                    && (token.is_start_tag() || token.is_character()))
                || token.is_eof()
            {
                // Process the token according to the rules given in the section corresponding
                // to the current insertion mode in HTML content.
                self.process_using_the_rules_for(self.insertion_mode, token);
            } else {
                // Process the token according to the rules given in the section for parsing
                // tokens in foreign content.
                self.process_using_the_rules_for_foreign_content(token);
            }
        }
    }

    fn process_using_the_rules_for(&mut self, mode: InsertionMode, token: &Token) {
        match mode {
            InsertionMode::Initial => self.handle_initial(token),
            InsertionMode::BeforeHTML => self.handle_before_html(token),
            InsertionMode::BeforeHead => self.handle_before_head(token),
            InsertionMode::InHead => self.handle_in_head(token),
            InsertionMode::InHeadNoscript => self.handle_in_head_noscript(token),
            InsertionMode::AfterHead => self.handle_after_head(token),
            InsertionMode::InBody => self.handle_in_body(token),
            InsertionMode::Text => self.handle_text(token),
            InsertionMode::InTable => self.handle_in_table(token),
            InsertionMode::InTableText => self.handle_in_table_text(token),
            InsertionMode::InCaption => self.handle_in_caption(token),
            InsertionMode::InColumnGroup => self.handle_in_column_group(token),
            InsertionMode::InTableBody => self.handle_in_table_body(token),
            InsertionMode::InRow => self.handle_in_row(token),
            InsertionMode::InCell => self.handle_in_cell(token),
            InsertionMode::InSelect => self.handle_in_select(token),
            InsertionMode::InSelectInTable => self.handle_in_select_in_table(token),
            InsertionMode::InTemplate => self.handle_in_template(token),
            InsertionMode::AfterBody => self.handle_after_body(token),
            InsertionMode::InFrameset => self.handle_in_frameset(token),
            InsertionMode::AfterFrameset => self.handle_after_frameset(token),
            InsertionMode::AfterAfterBody => self.handle_after_after_body(token),
            InsertionMode::AfterAfterFrameset => self.handle_after_after_frameset(token),
        }
    }

    // =======================================================================
    // Foreign content integration point helpers
    // =======================================================================

    /// https://html.spec.whatwg.org/multipage/parsing.html#adjusted-current-node
    fn adjusted_current_node(&self) -> Option<&StackEntry> {
        // The adjusted current node is the context element if the parser was created as part of the
        // HTML fragment parsing algorithm and the stack of open elements has only one element in it
        // (fragment case); otherwise, the adjusted current node is the current node.
        if self.parsing_fragment && self.stack_of_open_elements.len() == 1 {
            // In fragment case, we'd return the context element.
            // For now, return the current node.
            return self.stack_of_open_elements.current_node();
        }
        self.stack_of_open_elements.current_node()
    }

    fn adjusted_current_node_namespace(&self) -> Option<DomNamespace> {
        self.adjusted_current_node().map(|n| n.namespace)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#mathml-text-integration-point
    fn adjusted_current_node_is_mathml_text_integration_point(&self) -> bool {
        if let Some(node) = self.adjusted_current_node() {
            // A node is a MathML text integration point if it is one of the following elements:
            // A MathML mi, mo, mn, ms, or mtext element.
            node.namespace == DomNamespace::MathML
                && matches!(
                    node.tag_name.as_str(),
                    "mi" | "mo" | "mn" | "ms" | "mtext"
                )
        } else {
            false
        }
    }

    /// Check if the adjusted current node is a MathML annotation-xml element.
    fn adjusted_current_node_is_mathml_annotation_xml(&self) -> bool {
        if let Some(node) = self.adjusted_current_node() {
            node.namespace == DomNamespace::MathML && node.tag_name == "annotation-xml"
        } else {
            false
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#html-integration-point
    fn adjusted_current_node_is_html_integration_point(&self) -> bool {
        if let Some(node) = self.adjusted_current_node() {
            // A MathML annotation-xml element whose start tag token had an attribute with the name
            // "encoding" whose value was an ASCII case-insensitive match for "text/html" or
            // "application/xhtml+xml".
            // NOTE: We check the element's actual encoding attribute here.
            if node.namespace == DomNamespace::MathML && node.tag_name == "annotation-xml" {
                // TODO: Check encoding attribute on the actual DOM element
                return false;
            }
            // An SVG foreignObject, desc, or title element.
            if node.namespace == DomNamespace::SVG
                && matches!(node.tag_name.as_str(), "foreignObject" | "desc" | "title")
            {
                return true;
            }
        }
        false
    }

    // =======================================================================
    // Tree construction helpers
    // =======================================================================

    /// https://html.spec.whatwg.org/multipage/parsing.html#appropriate-place-for-inserting-a-node
    fn find_appropriate_place_for_inserting_node(
        &self,
        override_target: Option<DomHandle>,
    ) -> AdjustedInsertionLocation {
        // 1. If there was an override target specified, then let target be the override target.
        //    Otherwise, let target be the current node.
        let target = override_target.unwrap_or_else(|| {
            self.stack_of_open_elements
                .current_node()
                .expect("stack of open elements should not be empty")
                .handle
        });

        let target_tag = self
            .stack_of_open_elements
            .elements()
            .iter()
            .find(|e| e.handle == target)
            .map(|e| e.tag_name.as_str());

        let mut adjusted = AdjustedInsertionLocation {
            parent: target,
            insert_before_sibling: None,
        };

        // 2. Determine the adjusted insertion location using the first matching steps from the following list:

        // If foster parenting is enabled and target is a table, tbody, tfoot, thead, or tr element
        if self.foster_parenting
            && target_tag.is_some_and(|t| {
                matches!(t, "table" | "tbody" | "tfoot" | "thead" | "tr")
            })
        {
            // 1. Let last template be the last template element in the stack of open elements, if any.
            let last_template = self
                .stack_of_open_elements
                .last_element_with_tag_name("template");
            // 2. Let last table be the last table element in the stack of open elements, if any.
            let last_table = self
                .stack_of_open_elements
                .last_element_with_tag_name("table");

            // 3. If there is a last template and either there is no last table,
            //    or there is one, but last template is lower (more recently added) than last table
            if let Some((template_idx, template_entry)) = last_template {
                if last_table.is_none()
                    || last_table.is_some_and(|(table_idx, _)| template_idx > table_idx)
                {
                    // Let adjusted insertion location be inside last template's template contents,
                    // after its last child (if any).
                    return AdjustedInsertionLocation {
                        parent: template_entry.handle.template_contents(),
                        insert_before_sibling: None,
                    };
                }
            }

            // 4. If there is no last table, then let adjusted insertion location be inside the first
            //    element in the stack of open elements (the html element).
            if last_table.is_none() {
                return AdjustedInsertionLocation {
                    parent: self.stack_of_open_elements.first().unwrap().handle,
                    insert_before_sibling: None,
                };
            }

            let (_, table_entry) = last_table.unwrap();

            // 5. If last table has a parent node, then let adjusted insertion location be inside
            //    last table's parent node, immediately before last table.
            if let Some(parent) = table_entry.handle.parent() {
                return AdjustedInsertionLocation {
                    parent,
                    insert_before_sibling: Some(table_entry.handle),
                };
            }

            // 6. Let previous element be the element immediately above last table in the stack.
            if let Some(prev) = self
                .stack_of_open_elements
                .element_immediately_above(table_entry.handle)
            {
                adjusted = AdjustedInsertionLocation {
                    parent: prev.handle,
                    insert_before_sibling: None,
                };
            }
        } else {
            // Otherwise, let adjusted insertion location be inside target, after its last child.
            adjusted = AdjustedInsertionLocation {
                parent: target,
                insert_before_sibling: None,
            };
        }

        // 3. If the adjusted insertion location is inside a template element,
        //    let it instead be inside the template element's template contents.
        // NOTE: We check this by seeing if the parent is a template element on the stack.
        if let Some(entry) = self
            .stack_of_open_elements
            .elements()
            .iter()
            .find(|e| e.handle == adjusted.parent)
        {
            if entry.namespace == DomNamespace::HTML && entry.tag_name == "template" {
                adjusted = AdjustedInsertionLocation {
                    parent: entry.handle.template_contents(),
                    insert_before_sibling: None,
                };
            }
        }

        // 4. Return the adjusted insertion location.
        adjusted
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#insert-a-foreign-element
    fn insert_foreign_element(
        &mut self,
        token: &Token,
        namespace: DomNamespace,
        only_add_to_element_stack: bool,
    ) -> DomHandle {
        self.flush_character_insertions();
        // 1. Let the adjusted insertion location be the appropriate place for inserting a node.
        let adjusted = self.find_appropriate_place_for_inserting_node(None);

        // 2. Let element be the result of creating an element for the token in the given namespace.
        let element = self.create_element_for(token, namespace);

        // https://html.spec.whatwg.org/multipage/parsing.html#create-an-element-for-the-token
        // 11. Append each attribute in the given token to element.
        // NOTE: Using append_attribute ensures first attribute wins (per spec).
        for attr in token.attributes() {
            // Adjust foreign attributes (xlink:*, xml:*, xmlns:*).
            if let Some((ns, prefix, local_name)) = adjust_foreign_attribute(&attr.local_name) {
                element.set_attribute_ns(ns, prefix, local_name, &attr.value);
                continue;
            }
            // Adjust SVG attribute names if we're in the SVG namespace.
            if namespace == DomNamespace::SVG {
                let adjusted_name = adjust_svg_attribute_name(&attr.local_name);
                element.append_attribute(adjusted_name, &attr.value);
            } else {
                element.append_attribute(&attr.local_name, &attr.value);
            }
        }

        // Post-creation setup (e.g., media element muted attribute).
        unsafe {
            crate::dom_bridge::html_parser_bridge_post_create_element(element.as_ptr());
        }

        // 3. If onlyAddToElementStack is false, then run insert an element at the adjusted
        //    insertion location with element.
        if !only_add_to_element_stack {
            self.insert_at_adjusted_location(element, &adjusted);
        }

        // 4. Push element onto the stack of open elements so that it is the new current node.
        let tag_name = token.tag_name().to_string();
        self.stack_of_open_elements.push(StackEntry::new(
            element,
            tag_name,
            namespace,
        ));

        // 5. Return element.
        element
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#insert-an-html-element
    fn insert_html_element(&mut self, token: &Token) -> DomHandle {
        // When the steps below require the user agent to insert an HTML element for a token,
        // the user agent must insert a foreign element for the token, with the HTML namespace and false.
        self.insert_foreign_element(token, DomNamespace::HTML, false)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#create-an-element-for-the-token
    fn create_element_for(&self, token: &Token, namespace: DomNamespace) -> DomHandle {
        // Simplified version — create element via the bridge.
        DomHandle::create_element(self.document, token.tag_name(), namespace)
    }

    /// Insert a node at an adjusted insertion location.
    fn insert_at_adjusted_location(
        &self,
        node: DomHandle,
        location: &AdjustedInsertionLocation,
    ) {
        DomHandle::insert_before(location.parent, node, location.insert_before_sibling);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#insert-a-comment
    fn insert_comment(&mut self, token: &Token) {
        self.flush_character_insertions();
        let adjusted = self.find_appropriate_place_for_inserting_node(None);
        let comment = DomHandle::create_comment(self.document, token.comment_data());
        self.insert_at_adjusted_location(comment, &adjusted);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#insert-a-character
    fn insert_character(&mut self, code_point: u32) {
        let adjusted = self.find_appropriate_place_for_inserting_node(None);

        // If the adjusted insertion location is in a Document node, return.
        if adjusted.parent == DomHandle::document_node(self.document) {
            return;
        }

        // Find or create the text node we'll insert into.
        let text_node = if adjusted.insert_before_sibling.is_some() {
            // TODO: Check previous sibling of insert_before_sibling for an existing text node
            None::<DomHandle>
        } else {
            // Check if the last child of the parent is a text node.
            adjusted.parent.last_child().filter(|c| c.is_text_node())
        };

        let target_node = if let Some(node) = text_node {
            node
        } else {
            // We need to create a new text node. First flush any pending buffer.
            self.flush_character_insertions();
            let new_node = DomHandle::create_text_node(self.document, "");
            self.insert_at_adjusted_location(new_node, &adjusted);
            new_node
        };

        // If we're accumulating into a different node, flush the old buffer first.
        if let Some(existing) = self.character_insertion_node {
            if existing != target_node {
                self.flush_character_insertions();
            }
        }

        // Buffer the character.
        self.character_insertion_node = Some(target_node);
        if let Some(ch) = char::from_u32(code_point) {
            self.character_insertion_builder.push(ch);
        }
    }

    fn flush_character_insertions(&mut self) {
        if self.character_insertion_builder.is_empty() {
            self.character_insertion_node = None;
            return;
        }
        if let Some(node) = self.character_insertion_node {
            node.append_text(&self.character_insertion_builder);
        }
        self.character_insertion_builder.clear();
        self.character_insertion_node = None;
    }

    /// Helper to create a synthetic start tag token.
    fn make_start_tag_token(tag_name: &str) -> Token {
        Token {
            token_type: TokenType::StartTag,
            code_point: 0,
            payload: TokenPayload::Tag {
                tag_name: tag_name.to_string(),
                self_closing: false,
                attributes: Vec::new(),
            },
            start_position: Default::default(),
            end_position: Default::default(),
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#close-a-p-element
    fn close_a_p_element(&mut self) {
        // 1. Generate implied end tags, except for p elements.
        self.stack_of_open_elements
            .generate_implied_end_tags(Some("p"));
        // 2. If the current node is not a p element, then this is a parse error.
        // (We just log and continue.)
        // 3. Pop elements from the stack of open elements until a p element has been popped.
        self.stack_of_open_elements.pop_until_tag_name_popped("p");
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#generic-raw-text-element-parsing-algorithm
    fn parse_generic_raw_text_element(&mut self, token: &Token) {
        // 1. Insert an HTML element for the token.
        self.insert_html_element(token);
        // 2. Switch the tokenizer to the RAWTEXT state.
        self.tokenizer.switch_to(TokenizerState::RAWTEXT);
        // 3. Let the original insertion mode be the current insertion mode.
        self.original_insertion_mode = self.insertion_mode;
        // 4. Then, switch the insertion mode to "text".
        self.insertion_mode = InsertionMode::Text;
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#generic-rcdata-element-parsing-algorithm
    fn parse_generic_rcdata_element(&mut self, token: &Token) {
        // 1. Insert an HTML element for the token.
        self.insert_html_element(token);
        // 2. Switch the tokenizer to the RCDATA state.
        self.tokenizer.switch_to(TokenizerState::RCDATA);
        // 3. Let the original insertion mode be the current insertion mode.
        self.original_insertion_mode = self.insertion_mode;
        // 4. Then, switch the insertion mode to "text".
        self.insertion_mode = InsertionMode::Text;
    }

    /// Signal that the current token should be reprocessed.
    fn reprocess_token(&mut self) {
        self.reprocess = true;
    }

    fn stop_parsing(&mut self) {
        self.stop_parsing = true;
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#reconstruct-the-active-formatting-elements
    fn reconstruct_the_active_formatting_elements(&mut self) {
        // 1. If there are no entries in the list of active formatting elements, then there is
        //    nothing to reconstruct; stop this algorithm.
        if self.list_of_active_formatting_elements.is_empty() {
            return;
        }

        let entries = self.list_of_active_formatting_elements.entries();

        // 2. If the last (most recently added) entry in the list of active formatting elements
        //    is a marker, or if it is an element that is in the stack of open elements,
        //    then there is nothing to reconstruct; stop this algorithm.
        let last = entries.last().unwrap();
        if last.is_marker() {
            return;
        }
        if let Some(handle) = last.handle() {
            if self.stack_of_open_elements.contains(handle) {
                return;
            }
        }

        // 3. Let entry be the last (most recently added) element in the list.
        let mut index = entries.len() - 1;

        // 4. Rewind: If there are no entries before entry in the list, then jump to Create.
        loop {
            if index == 0 {
                break; // Jump to Create
            }

            // 5. Let entry be the entry one earlier than entry.
            index -= 1;

            let entry = &self.list_of_active_formatting_elements.entries()[index];

            // 6. If entry is neither a marker nor an element that is also in the stack of open
            //    elements, go to the step labeled Rewind.
            if entry.is_marker() {
                index += 1;
                break;
            }
            if let Some(handle) = entry.handle() {
                if self.stack_of_open_elements.contains(handle) {
                    index += 1;
                    break;
                }
            }
        }

        // 7. Advance: Let entry be the element one later than entry.
        // 8. Create: Insert an HTML element for the token for which the element entry was created.
        loop {
            let token = {
                let entry = &self.list_of_active_formatting_elements.entries()[index];
                match entry {
                    FormattingEntry::Element { token, .. } => token.clone(),
                    FormattingEntry::Marker => unreachable!(),
                }
            };

            let new_element = self.insert_html_element(&token);
            let tag_name = token.tag_name().to_string();

            // Replace the entry for the new element in the list.
            let entries = self.list_of_active_formatting_elements.entries_mut();
            entries[index] = FormattingEntry::Element {
                handle: new_element,
                tag_name,
                token,
            };

            // If the entry is not the last entry in the list, return to Advance.
            index += 1;
            if index >= self.list_of_active_formatting_elements.entries().len() {
                break;
            }
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#reset-the-insertion-mode-appropriately
    fn reset_the_insertion_mode_appropriately(&mut self) {
        // 1. Let last be false.
        let mut last = false;
        // 2. Let node be the last node in the stack of open elements.
        let mut node_index = self.stack_of_open_elements.len() - 1;

        loop {
            let node = self.stack_of_open_elements.entry_at(node_index);
            let tag = node.tag_name.as_str();
            let ns = node.namespace;

            // 3. Loop: If node is the first node in the stack of open elements, then set last to
            //    true, and, if the parser was created as part of the HTML fragment parsing algorithm
            //    (fragment case), set node to the context element passed to that algorithm.
            if node_index == 0 {
                last = true;
                // TODO: fragment case - set node to context element
            }

            if ns == DomNamespace::HTML {
                // 4. If node is a select element, set the insertion mode to InBody.
                // NOTE: The current HTML spec no longer has InSelect mode.
                if tag == "select" {
                    self.insertion_mode = InsertionMode::InBody;
                    return;
                }

                // 5. If node is a td or th element and last is false, ...
                if (tag == "td" || tag == "th") && !last {
                    self.insertion_mode = InsertionMode::InCell;
                    return;
                }

                // 6. If node is a tr element, ...
                if tag == "tr" {
                    self.insertion_mode = InsertionMode::InRow;
                    return;
                }

                // 7. If node is a tbody, thead, or tfoot element, ...
                if matches!(tag, "tbody" | "thead" | "tfoot") {
                    self.insertion_mode = InsertionMode::InTableBody;
                    return;
                }

                // 8. If node is a caption element, ...
                if tag == "caption" {
                    self.insertion_mode = InsertionMode::InCaption;
                    return;
                }

                // 9. If node is a colgroup element, ...
                if tag == "colgroup" {
                    self.insertion_mode = InsertionMode::InColumnGroup;
                    return;
                }

                // 10. If node is a table element, ...
                if tag == "table" {
                    self.insertion_mode = InsertionMode::InTable;
                    return;
                }

                // 11. If node is a template element, ...
                if tag == "template" {
                    self.insertion_mode = *self
                        .stack_of_template_insertion_modes
                        .last()
                        .unwrap_or(&InsertionMode::InTemplate);
                    return;
                }

                // 12. If node is a head element and last is false, ...
                if tag == "head" && !last {
                    self.insertion_mode = InsertionMode::InHead;
                    return;
                }

                // 13. If node is a body element, ...
                if tag == "body" {
                    self.insertion_mode = InsertionMode::InBody;
                    return;
                }

                // 14. If node is a frameset element, ...
                if tag == "frameset" {
                    self.insertion_mode = InsertionMode::InFrameset;
                    return;
                }

                // 15. If node is an html element, ...
                if tag == "html" {
                    if self.head_element.is_none() {
                        self.insertion_mode = InsertionMode::BeforeHead;
                    } else {
                        self.insertion_mode = InsertionMode::AfterHead;
                    }
                    return;
                }
            }

            // 16. If last is true, ...
            if last {
                self.insertion_mode = InsertionMode::InBody;
                return;
            }

            // 17. Let node now be the node before node in the stack of open elements.
            if node_index == 0 {
                break;
            }
            node_index -= 1;
        }
    }

    // =======================================================================
    // Quirks mode determination
    // https://html.spec.whatwg.org/multipage/parsing.html#the-initial-insertion-mode
    // =======================================================================

    fn which_quirks_mode(&self, token: &Token) -> QuirksMode {
        let doctype = token.doctype_data();

        if doctype.force_quirks {
            return QuirksMode::Yes;
        }

        // NOTE: The tokenizer puts the name into lower case for us.
        if doctype.name != "html" {
            return QuirksMode::Yes;
        }

        let public_id = &doctype.public_identifier;
        let system_id = &doctype.system_identifier;

        if public_id.eq_ignore_ascii_case("-//W3O//DTD W3 HTML Strict 3.0//EN//") {
            return QuirksMode::Yes;
        }
        if public_id.eq_ignore_ascii_case("-/W3C/DTD HTML 4.0 Transitional/EN") {
            return QuirksMode::Yes;
        }
        if public_id.eq_ignore_ascii_case("HTML") {
            return QuirksMode::Yes;
        }
        if system_id.eq_ignore_ascii_case(
            "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd",
        ) {
            return QuirksMode::Yes;
        }

        for prefix in QUIRKS_PUBLIC_ID_PREFIXES {
            if public_id
                .get(..prefix.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
            {
                return QuirksMode::Yes;
            }
        }

        if doctype.missing_system_identifier {
            if public_id
                .get(.."-//W3C//DTD HTML 4.01 Frameset//".len())
                .is_some_and(|s| s.eq_ignore_ascii_case("-//W3C//DTD HTML 4.01 Frameset//"))
            {
                return QuirksMode::Yes;
            }
            if public_id
                .get(.."-//W3C//DTD HTML 4.01 Transitional//".len())
                .is_some_and(|s| {
                    s.eq_ignore_ascii_case("-//W3C//DTD HTML 4.01 Transitional//")
                })
            {
                return QuirksMode::Yes;
            }
        }

        if public_id
            .get(.."-//W3C//DTD XHTML 1.0 Frameset//".len())
            .is_some_and(|s| s.eq_ignore_ascii_case("-//W3C//DTD XHTML 1.0 Frameset//"))
        {
            return QuirksMode::Limited;
        }
        if public_id
            .get(.."-//W3C//DTD XHTML 1.0 Transitional//".len())
            .is_some_and(|s| s.eq_ignore_ascii_case("-//W3C//DTD XHTML 1.0 Transitional//"))
        {
            return QuirksMode::Limited;
        }

        if !doctype.missing_system_identifier {
            if public_id
                .get(.."-//W3C//DTD HTML 4.01 Frameset//".len())
                .is_some_and(|s| s.eq_ignore_ascii_case("-//W3C//DTD HTML 4.01 Frameset//"))
            {
                return QuirksMode::Limited;
            }
            if public_id
                .get(.."-//W3C//DTD HTML 4.01 Transitional//".len())
                .is_some_and(|s| {
                    s.eq_ignore_ascii_case("-//W3C//DTD HTML 4.01 Transitional//")
                })
            {
                return QuirksMode::Limited;
            }
        }

        QuirksMode::No
    }

    // =======================================================================
    // Insertion mode handlers
    // =======================================================================

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-initial-insertion-mode
    fn handle_initial(&mut self, token: &Token) {
        // -> A character token that is one of U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
        //    U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), or U+0020 SPACE
        if token.is_parser_whitespace() {
            // Ignore the token.
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment as the last child of the Document object.
            let comment = DomHandle::create_comment(self.document, token.comment_data());
            let doc_node = DomHandle::document_node(self.document);
            DomHandle::insert_before(doc_node, comment, None);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            let doctype = token.doctype_data();

            // If the DOCTYPE token's name is not "html", or the token's public identifier is not
            // missing, or the token's system identifier is neither missing nor "about:legacy-compat",
            // then there is a parse error.
            // (We just log and continue.)

            // Append a DocumentType node to the Document node.
            let name = if doctype.missing_name {
                ""
            } else {
                &doctype.name
            };
            let public_id = if doctype.missing_public_identifier {
                ""
            } else {
                &doctype.public_identifier
            };
            let system_id = if doctype.missing_system_identifier {
                ""
            } else {
                &doctype.system_identifier
            };
            DomHandle::insert_doctype(self.document, name, public_id, system_id);

            // Set the Document to quirks mode if appropriate.
            let quirks = self.which_quirks_mode(token);
            DomHandle::set_quirks_mode(self.document, quirks);

            // Then, switch the insertion mode to "before html".
            self.insertion_mode = InsertionMode::BeforeHTML;
            return;
        }

        // -> Anything else
        // Parse error. Set the Document to quirks mode.
        DomHandle::set_quirks_mode(self.document, QuirksMode::Yes);

        // Switch the insertion mode to "before html" and reprocess the token.
        self.insertion_mode = InsertionMode::BeforeHTML;
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-before-html-insertion-mode
    fn handle_before_html(&mut self, token: &Token) {
        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment as the last child of the Document object.
            let comment = DomHandle::create_comment(self.document, token.comment_data());
            let doc_node = DomHandle::document_node(self.document);
            DomHandle::insert_before(doc_node, comment, None);
            return;
        }

        // -> A character token that is one of U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
        //    U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), or U+0020 SPACE
        if token.is_parser_whitespace() {
            // Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Create an element for the token in the HTML namespace, with the Document as the
            // intended parent. Append it to the Document object. Put this element in the stack
            // of open elements.
            let element = self.create_element_for(token, DomNamespace::HTML);
            for attr in token.attributes() {
                element.append_attribute(&attr.local_name, &attr.value);
            }
            let doc_node = DomHandle::document_node(self.document);
            DomHandle::insert_before(doc_node, element, None);
            self.stack_of_open_elements.push(StackEntry::new(
                element,
                "html".to_string(),
                DomNamespace::HTML,
            ));

            // Switch the insertion mode to "before head".
            self.insertion_mode = InsertionMode::BeforeHead;
            return;
        }

        // -> An end tag whose tag name is one of: "head", "body", "html", "br"
        if token.is_end_tag()
            && matches!(token.tag_name(), "head" | "body" | "html" | "br")
        {
            // Act as described in the "anything else" entry below.
            // (Fall through to anything else.)
        } else if token.is_end_tag() {
            // -> Any other end tag
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        // Create an html element whose node document is the Document object.
        // Append it to the Document object. Put this element in the stack of open elements.
        let element = DomHandle::create_element(self.document, "html", DomNamespace::HTML);
        let doc_node = DomHandle::document_node(self.document);
        DomHandle::insert_before(doc_node, element, None);
        self.stack_of_open_elements.push(StackEntry::new(
            element,
            "html".to_string(),
            DomNamespace::HTML,
        ));

        // Switch the insertion mode to "before head", then reprocess the token.
        self.insertion_mode = InsertionMode::BeforeHead;
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-before-head-insertion-mode
    fn handle_before_head(&mut self, token: &Token) {
        // -> A character token that is one of U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
        //    U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), or U+0020 SPACE
        if token.is_parser_whitespace() {
            // Ignore the token.
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment.
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is "head"
        if token.is_start_tag() && token.tag_name() == "head" {
            // Insert an HTML element for the token.
            let element = self.insert_html_element(token);

            // Set the head element pointer to the newly created head element.
            self.head_element = Some(element);

            // Switch the insertion mode to "in head".
            self.insertion_mode = InsertionMode::InHead;
            return;
        }

        // -> An end tag whose tag name is one of: "head", "body", "html", "br"
        if token.is_end_tag()
            && matches!(token.tag_name(), "head" | "body" | "html" | "br")
        {
            // Act as described in the "anything else" entry below.
            // (Fall through.)
        } else if token.is_end_tag() {
            // -> Any other end tag
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        // Insert an HTML element for a "head" start tag token with no attributes.
        let head_token = Self::make_start_tag_token("head");
        let element = self.insert_html_element(&head_token);

        // Set the head element pointer to the newly created head element.
        self.head_element = Some(element);

        // Switch the insertion mode to "in head".
        self.insertion_mode = InsertionMode::InHead;

        // Reprocess the current token.
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inhead
    fn handle_in_head(&mut self, token: &Token) {
        // -> A character token that is one of U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
        //    U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), or U+0020 SPACE
        if token.is_parser_whitespace() {
            // Insert the character.
            self.insert_character(token.code_point);
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment.
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is one of: "base", "basefont", "bgsound", "link"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "base" | "basefont" | "bgsound" | "link"
            )
        {
            // Insert an HTML element for the token. Immediately pop the current node off the stack.
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            // Acknowledge the token's self-closing flag, if it is set.
            return;
        }

        // -> A start tag whose tag name is "meta"
        if token.is_start_tag() && token.tag_name() == "meta" {
            // Insert an HTML element for the token. Immediately pop the current node off the stack.
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            // Acknowledge the token's self-closing flag, if it is set.
            return;
        }

        // -> A start tag whose tag name is "title"
        if token.is_start_tag() && token.tag_name() == "title" {
            // Follow the generic RCDATA element parsing algorithm.
            self.parse_generic_rcdata_element(token);
            return;
        }

        // -> A start tag whose tag name is "noscript", if the scripting flag is enabled
        // -> A start tag whose tag name is one of: "noframes", "style"
        if token.is_start_tag()
            && ((token.tag_name() == "noscript" && self.scripting_enabled)
                || matches!(token.tag_name(), "noframes" | "style"))
        {
            // Follow the generic raw text element parsing algorithm.
            self.parse_generic_raw_text_element(token);
            return;
        }

        // -> A start tag whose tag name is "noscript", if the scripting flag is disabled
        if token.is_start_tag() && token.tag_name() == "noscript" && !self.scripting_enabled {
            // Insert an HTML element for the token.
            self.insert_html_element(token);
            // Switch the insertion mode to "in head noscript".
            self.insertion_mode = InsertionMode::InHeadNoscript;
            return;
        }

        // -> A start tag whose tag name is "script"
        if token.is_start_tag() && token.tag_name() == "script" {
            // 1. Let the adjusted insertion location be the appropriate place for inserting a node.
            let adjusted = self.find_appropriate_place_for_inserting_node(None);

            // 2. Create an element for the token in the HTML namespace.
            let element = self.create_element_for(token, DomNamespace::HTML);
            for attr in token.attributes() {
                element.append_attribute(&attr.local_name, &attr.value);
            }

            // 3. Set the element's parser document to the Document, and set the element's
            //    force async to false.
            element.setup_script_element(self.document);

            // 6. Insert the newly created element at the adjusted insertion location.
            self.insert_at_adjusted_location(element, &adjusted);

            // 7. Push the element onto the stack of open elements.
            self.stack_of_open_elements.push(StackEntry::new(
                element,
                "script".to_string(),
                DomNamespace::HTML,
            ));

            // 8. Switch the tokenizer to the script data state.
            self.tokenizer.switch_to(TokenizerState::ScriptData);

            // 9. Set the original insertion mode to the current insertion mode.
            self.original_insertion_mode = self.insertion_mode;

            // 10. Switch the insertion mode to "text".
            self.insertion_mode = InsertionMode::Text;
            return;
        }

        // -> An end tag whose tag name is "head"
        if token.is_end_tag() && token.tag_name() == "head" {
            // Pop the current node (which will be the head element) off the stack.
            self.stack_of_open_elements.pop();

            // Switch the insertion mode to "after head".
            self.insertion_mode = InsertionMode::AfterHead;
            return;
        }

        // -> An end tag whose tag name is one of: "body", "html", "br"
        if token.is_end_tag() && matches!(token.tag_name(), "body" | "html" | "br") {
            // Act as described in the "anything else" entry below.
            // (Fall through.)
        }
        // -> A start tag whose tag name is "template"
        else if token.is_start_tag() && token.tag_name() == "template" {
            // Insert a marker at the end of the list of active formatting elements.
            self.list_of_active_formatting_elements.add_marker();

            // Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;

            // Switch the insertion mode to "in template".
            self.insertion_mode = InsertionMode::InTemplate;

            // Push "in template" onto the stack of template insertion modes.
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InTemplate);

            // Check for declarative shadow root (shadowrootmode attribute).
            let shadowrootmode = token.get_attribute("shadowrootmode");
            let has_valid_shadowrootmode = shadowrootmode
                .is_some_and(|m| m == "open" || m == "closed");

            // 9. If shadowrootmode is not valid, or document doesn't allow declarative shadow
            //    roots, or the adjusted current node IS the topmost element: insert normally.
            if !has_valid_shadowrootmode
                || self.stack_of_open_elements.len() <= 1
            {
                // Normal template: insert an HTML element for the token.
                self.insert_html_element(token);
            } else {
                // 10. Otherwise: declarative shadow DOM.
                let host_handle = self.adjusted_current_node().unwrap().handle;

                // Insert a foreign element with only_add_to_element_stack=true.
                let template_element =
                    self.insert_foreign_element(token, DomNamespace::HTML, true);

                let mode = shadowrootmode.unwrap();
                let clonable = token.has_attribute("shadowrootclonable");
                let serializable = token.has_attribute("shadowrootserializable");
                let delegates_focus = token.has_attribute("shadowrootdelegatesfocus");

                let success = unsafe {
                    crate::dom_bridge::html_parser_bridge_handle_declarative_shadow_template(
                        template_element.as_ptr(),
                        host_handle.as_ptr(),
                        self.document.as_ptr(),
                        mode.as_ptr(),
                        mode.len(),
                        clonable,
                        serializable,
                        delegates_focus,
                    )
                };

                if !success {
                    // Shadow root attachment failed: insert the template at the adjusted location.
                    let adjusted = self.find_appropriate_place_for_inserting_node(None);
                    self.insert_at_adjusted_location(template_element, &adjusted);
                }
            }
            return;
        }
        // -> An end tag whose tag name is "template"
        else if token.is_end_tag() && token.tag_name() == "template" {
            // If there is no template element on the stack of open elements, then this is a
            // parse error; ignore the token.
            if !self.stack_of_open_elements.contains_template_element() {
                return;
            }

            // 1. Generate all implied end tags thoroughly.
            self.stack_of_open_elements
                .generate_all_implied_end_tags_thoroughly();

            // 2. If the current node is not a template element, then this is a parse error.

            // 3. Pop elements from the stack until a template element has been popped.
            self.stack_of_open_elements
                .pop_until_tag_name_popped("template");

            // 4. Clear the list of active formatting elements up to the last marker.
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();

            // 5. Pop the current template insertion mode off the stack.
            self.stack_of_template_insertion_modes.pop();

            // 6. Reset the insertion mode appropriately.
            self.reset_the_insertion_mode_appropriately();
            return;
        }
        // -> A start tag whose tag name is "head"
        // -> Any other end tag
        else if (token.is_start_tag() && token.tag_name() == "head") || token.is_end_tag() {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        // Pop the current node (which will be the head element) off the stack.
        self.stack_of_open_elements.pop();

        // Switch the insertion mode to "after head".
        self.insertion_mode = InsertionMode::AfterHead;

        // Reprocess the token.
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inheadnoscript
    fn handle_in_head_noscript(&mut self, token: &Token) {
        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> An end tag whose tag name is "noscript"
        if token.is_end_tag() && token.tag_name() == "noscript" {
            // Pop the current node (which will be a noscript element) from the stack.
            self.stack_of_open_elements.pop();

            // Switch the insertion mode to "in head".
            self.insertion_mode = InsertionMode::InHead;
            return;
        }

        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        // -> A comment token
        // -> A start tag whose tag name is one of: "basefont", "bgsound", "link", "meta",
        //    "noframes", "style"
        if token.is_parser_whitespace()
            || token.is_comment()
            || (token.is_start_tag()
                && matches!(
                    token.tag_name(),
                    "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
                ))
        {
            // Process the token using the rules for the "in head" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> An end tag whose tag name is "br"
        if token.is_end_tag() && token.tag_name() == "br" {
            // Act as described in the "anything else" entry below.
            // (Fall through.)
        }
        // -> A start tag whose tag name is one of: "head", "noscript"
        // -> Any other end tag
        else if (token.is_start_tag()
            && matches!(token.tag_name(), "head" | "noscript"))
            || token.is_end_tag()
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        // Parse error.

        // Pop the current node (which will be a noscript element) from the stack.
        self.stack_of_open_elements.pop();

        // Switch the insertion mode to "in head".
        self.insertion_mode = InsertionMode::InHead;

        // Reprocess the token.
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-after-head-insertion-mode
    fn handle_after_head(&mut self, token: &Token) {
        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        if token.is_parser_whitespace() {
            // Insert the character.
            self.insert_character(token.code_point);
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment.
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is "body"
        if token.is_start_tag() && token.tag_name() == "body" {
            // Insert an HTML element for the token.
            self.insert_html_element(token);

            // Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;

            // Switch the insertion mode to "in body".
            self.insertion_mode = InsertionMode::InBody;
            return;
        }

        // -> A start tag whose tag name is "frameset"
        if token.is_start_tag() && token.tag_name() == "frameset" {
            // Insert an HTML element for the token.
            self.insert_html_element(token);

            // Switch the insertion mode to "in frameset".
            self.insertion_mode = InsertionMode::InFrameset;
            return;
        }

        // -> A start tag whose tag name is one of: "base", "basefont", "bgsound", "link",
        //    "meta", "noframes", "script", "style", "template", "title"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            )
        {
            // Parse error.

            // Push the node pointed to by the head element pointer onto the stack.
            if let Some(head) = self.head_element {
                self.stack_of_open_elements.push(StackEntry::new(
                    head,
                    "head".to_string(),
                    DomNamespace::HTML,
                ));
            }

            // Process the token using the rules for the "in head" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InHead, token);

            // Remove the node pointed to by the head element pointer from the stack.
            if let Some(head) = self.head_element {
                self.stack_of_open_elements.remove(head);
            }
            return;
        }

        // -> An end tag whose tag name is "template"
        if token.is_end_tag() && token.tag_name() == "template" {
            // Process the token using the rules for the "in head" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> An end tag whose tag name is one of: "body", "html", "br"
        if token.is_end_tag() && matches!(token.tag_name(), "body" | "html" | "br") {
            // Act as described in the "anything else" entry below.
            // (Fall through.)
        }
        // -> A start tag whose tag name is "head"
        // -> Any other end tag
        else if (token.is_start_tag() && token.tag_name() == "head") || token.is_end_tag() {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        // Insert an HTML element for a "body" start tag token with no attributes.
        let body_token = Self::make_start_tag_token("body");
        self.insert_html_element(&body_token);

        // Switch the insertion mode to "in body".
        self.insertion_mode = InsertionMode::InBody;

        // Reprocess the current token.
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inbody
    fn handle_in_body(&mut self, token: &Token) {
        // -> A character token that is U+0000 NULL
        if token.is_character() && token.code_point == 0 {
            // Parse error. Ignore the token.
            return;
        }

        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        if token.is_parser_whitespace() {
            // Reconstruct the active formatting elements, if any.
            self.reconstruct_the_active_formatting_elements();
            // Insert the character.
            self.insert_character(token.code_point);
            return;
        }

        // -> Any other character token
        if token.is_character() {
            // Reconstruct the active formatting elements, if any.
            self.reconstruct_the_active_formatting_elements();
            // Insert the character.
            self.insert_character(token.code_point);
            // Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment.
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Parse error.
            // If there is a template element on the stack of open elements, then ignore the token.
            if self.stack_of_open_elements.contains_template_element() {
                return;
            }
            // Otherwise, for each attribute on the token, check if the attribute is already present
            // on the top element of the stack. If it is not, add the attribute and its corresponding
            // value to that element.
            if let Some(first) = self.stack_of_open_elements.first() {
                let handle = first.handle;
                for attr in token.attributes() {
                    // append_attribute only adds if not already present (first one wins).
                    handle.append_attribute(&attr.local_name, &attr.value);
                }
            }
            return;
        }

        // -> A start tag whose tag name is one of: "base", "basefont", "bgsound", "link", "meta",
        //    "noframes", "script", "style", "template", "title"
        // -> An end tag whose tag name is "template"
        if (token.is_start_tag()
            && matches!(
                token.tag_name(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            ))
            || (token.is_end_tag() && token.tag_name() == "template")
        {
            // Process the token using the rules for the "in head" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> A start tag whose tag name is "body"
        if token.is_start_tag() && token.tag_name() == "body" {
            // Parse error.
            // If the stack of open elements has only one node on it, or if the second element on
            // the stack is not a body element, ignore the token. (fragment case)
            if self.stack_of_open_elements.len() <= 1 {
                return;
            }
            if let Some(second) = self.stack_of_open_elements.get(1) {
                if !second.is_html_element("body") {
                    return;
                }
            }
            // If there is a template element on the stack, ignore the token.
            if self.stack_of_open_elements.contains_template_element() {
                return;
            }
            // Otherwise, set the frameset-ok flag to "not ok".
            self.frameset_ok = false;
            // For each attribute on the token, check if the attribute is already present on the
            // body element. If not, add it.
            if let Some(body) = self.stack_of_open_elements.get(1) {
                let handle = body.handle;
                for attr in token.attributes() {
                    handle.append_attribute(&attr.local_name, &attr.value);
                }
            }
            return;
        }

        // -> A start tag whose tag name is "frameset"
        if token.is_start_tag() && token.tag_name() == "frameset" {
            // Parse error.
            if self.stack_of_open_elements.len() <= 1 {
                return;
            }
            if let Some(second) = self.stack_of_open_elements.get(1) {
                if !second.is_html_element("body") {
                    return;
                }
            }
            if !self.frameset_ok {
                return;
            }
            // TODO: Remove the second element from the stack and its parent,
            // then insert frameset element.
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            // If the stack of template insertion modes is not empty, then process the token using
            // the rules for the "in template" insertion mode.
            if !self.stack_of_template_insertion_modes.is_empty() {
                self.process_using_the_rules_for(InsertionMode::InTemplate, token);
                return;
            }
            // Stop parsing.
            self.stop_parsing();
            return;
        }

        // -> An end tag whose tag name is "body"
        if token.is_end_tag() && token.tag_name() == "body" {
            // If the stack of open elements does not have a body element in scope, this is a
            // parse error; ignore the token.
            if !self.stack_of_open_elements.has_in_scope("body") {
                return;
            }
            // Switch the insertion mode to "after body".
            self.insertion_mode = InsertionMode::AfterBody;
            return;
        }

        // -> An end tag whose tag name is "html"
        if token.is_end_tag() && token.tag_name() == "html" {
            // If the stack of open elements does not have a body element in scope, this is a
            // parse error; ignore the token.
            if !self.stack_of_open_elements.has_in_scope("body") {
                return;
            }
            // Switch the insertion mode to "after body".
            self.insertion_mode = InsertionMode::AfterBody;
            // Reprocess the token.
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is one of: "address", "article", "aside", "blockquote",
        //    "center", "details", "dialog", "dir", "div", "dl", "fieldset", "figcaption",
        //    "figure", "footer", "header", "hgroup", "main", "menu", "nav", "ol", "p",
        //    "search", "section", "summary", "ul"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "center"
                    | "details"
                    | "dialog"
                    | "dir"
                    | "div"
                    | "dl"
                    | "fieldset"
                    | "figcaption"
                    | "figure"
                    | "footer"
                    | "header"
                    | "hgroup"
                    | "main"
                    | "menu"
                    | "nav"
                    | "ol"
                    | "p"
                    | "search"
                    | "section"
                    | "summary"
                    | "ul"
            )
        {
            // If the stack of open elements has a p element in button scope, then close a p element.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            // Insert an HTML element for the token.
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is one of: "h1", "h2", "h3", "h4", "h5", "h6"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            )
        {
            // If the stack of open elements has a p element in button scope, then close a p element.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            // If the current node is an HTML element whose tag name is one of "h1"-"h6",
            // parse error, pop the current node.
            if let Some(current) = self.stack_of_open_elements.current_node() {
                if current.namespace == DomNamespace::HTML
                    && matches!(
                        current.tag_name.as_str(),
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    )
                {
                    self.stack_of_open_elements.pop();
                }
            }
            // Insert an HTML element for the token.
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is one of: "pre", "listing"
        if token.is_start_tag() && matches!(token.tag_name(), "pre" | "listing") {
            // If the stack of open elements has a p element in button scope, then close a p element.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            // Insert an HTML element for the token.
            self.insert_html_element(token);
            // Set the next token LF ignore flag.
            self.next_line_feed_can_be_ignored = true;
            // Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;
            return;
        }

        // -> A start tag whose tag name is "form"
        if token.is_start_tag() && token.tag_name() == "form" {
            // If the form element pointer is not null, and there is no template element on the
            // stack of open elements, then this is a parse error; ignore the token.
            if self.form_element.is_some()
                && !self.stack_of_open_elements.contains_template_element()
            {
                return;
            }
            // If the stack of open elements has a p element in button scope, then close a p element.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            // Insert an HTML element for the token.
            let element = self.insert_html_element(token);
            // If there is no template element on the stack, set the form element pointer.
            if !self.stack_of_open_elements.contains_template_element() {
                self.form_element = Some(element);
            }
            return;
        }

        // -> A start tag whose tag name is "li"
        if token.is_start_tag() && token.tag_name() == "li" {
            // 1. Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;

            // 2. Initialize node to be the current node (the bottommost node of the stack).
            for i in (0..self.stack_of_open_elements.len()).rev() {
                let entry = self.stack_of_open_elements.entry_at(i);
                // 3. Loop: If node is an li element, then run these substeps:
                if entry.is_html_element("li") {
                    // Generate implied end tags, except for li elements.
                    self.stack_of_open_elements
                        .generate_implied_end_tags(Some("li"));
                    // Pop elements until an li element has been popped.
                    self.stack_of_open_elements.pop_until_tag_name_popped("li");
                    break;
                }
                // If node is in the special category, but not an address, div, or p element,
                // then jump to the step labeled done below.
                if is_special_tag(&entry.tag_name, entry.namespace)
                    && !matches!(entry.tag_name.as_str(), "address" | "div" | "p")
                {
                    break;
                }
            }

            // If the stack of open elements has a p element in button scope, close a p element.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            // Insert an HTML element for the token.
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is one of: "dd", "dt"
        if token.is_start_tag() && matches!(token.tag_name(), "dd" | "dt") {
            // 1. Set the frameset-ok flag to "not ok".
            self.frameset_ok = false;

            for i in (0..self.stack_of_open_elements.len()).rev() {
                let entry = self.stack_of_open_elements.entry_at(i);
                if entry.is_html_element("dd") {
                    self.stack_of_open_elements
                        .generate_implied_end_tags(Some("dd"));
                    self.stack_of_open_elements
                        .pop_until_tag_name_popped("dd");
                    break;
                }
                if entry.is_html_element("dt") {
                    self.stack_of_open_elements
                        .generate_implied_end_tags(Some("dt"));
                    self.stack_of_open_elements
                        .pop_until_tag_name_popped("dt");
                    break;
                }
                if is_special_tag(&entry.tag_name, entry.namespace)
                    && !matches!(entry.tag_name.as_str(), "address" | "div" | "p")
                {
                    break;
                }
            }

            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is "plaintext"
        if token.is_start_tag() && token.tag_name() == "plaintext" {
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            self.insert_html_element(token);
            self.tokenizer.switch_to(TokenizerState::PLAINTEXT);
            return;
        }

        // -> A start tag whose tag name is "button"
        if token.is_start_tag() && token.tag_name() == "button" {
            if self.stack_of_open_elements.has_in_scope("button") {
                // Parse error.
                self.stack_of_open_elements
                    .generate_implied_end_tags(None);
                self.stack_of_open_elements
                    .pop_until_tag_name_popped("button");
            }
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            self.frameset_ok = false;
            return;
        }

        // -> An end tag whose tag name is one of: "address", "article", "aside", "blockquote",
        //    "button", "center", "details", "dialog", "dir", "div", "dl", "fieldset",
        //    "figcaption", "figure", "footer", "header", "hgroup", "listing", "main", "menu",
        //    "nav", "ol", "pre", "search", "section", "summary", "ul"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "button"
                    | "center"
                    | "details"
                    | "dialog"
                    | "dir"
                    | "div"
                    | "dl"
                    | "fieldset"
                    | "figcaption"
                    | "figure"
                    | "footer"
                    | "header"
                    | "hgroup"
                    | "listing"
                    | "main"
                    | "menu"
                    | "nav"
                    | "ol"
                    | "pre"
                    | "search"
                    | "section"
                    | "summary"
                    | "ul"
            )
        {
            // If the stack of open elements does not have an element in scope that is an HTML
            // element with the same tag name as that of the token, then this is a parse error;
            // ignore the token.
            if !self.stack_of_open_elements.has_in_scope(token.tag_name()) {
                return;
            }
            // Generate implied end tags.
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            // Pop elements until an element with the same tag name has been popped.
            self.stack_of_open_elements
                .pop_until_tag_name_popped(token.tag_name());
            return;
        }

        // -> An end tag whose tag name is "form"
        if token.is_end_tag() && token.tag_name() == "form" {
            if !self.stack_of_open_elements.contains_template_element() {
                let node = self.form_element.take();
                if node.is_none() {
                    return;
                }
                let node = node.unwrap();
                if !self.stack_of_open_elements.has_element_in_scope(node) {
                    return;
                }
                self.stack_of_open_elements
                    .generate_implied_end_tags(None);
                self.stack_of_open_elements.remove(node);
            } else {
                if !self.stack_of_open_elements.has_in_scope("form") {
                    return;
                }
                self.stack_of_open_elements
                    .generate_implied_end_tags(None);
                self.stack_of_open_elements
                    .pop_until_tag_name_popped("form");
            }
            return;
        }

        // -> An end tag whose tag name is "p"
        if token.is_end_tag() && token.tag_name() == "p" {
            if !self.stack_of_open_elements.has_in_button_scope("p") {
                // Parse error.
                let p_token = Self::make_start_tag_token("p");
                self.insert_html_element(&p_token);
            }
            self.close_a_p_element();
            return;
        }

        // -> An end tag whose tag name is "li"
        if token.is_end_tag() && token.tag_name() == "li" {
            if !self.stack_of_open_elements.has_in_list_item_scope("li") {
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(Some("li"));
            self.stack_of_open_elements
                .pop_until_tag_name_popped("li");
            return;
        }

        // -> An end tag whose tag name is one of: "dd", "dt"
        if token.is_end_tag() && matches!(token.tag_name(), "dd" | "dt") {
            if !self.stack_of_open_elements.has_in_scope(token.tag_name()) {
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(Some(token.tag_name()));
            self.stack_of_open_elements
                .pop_until_tag_name_popped(token.tag_name());
            return;
        }

        // -> An end tag whose tag name is one of: "h1", "h2", "h3", "h4", "h5", "h6"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            )
        {
            if !self.stack_of_open_elements.has_in_scope("h1")
                && !self.stack_of_open_elements.has_in_scope("h2")
                && !self.stack_of_open_elements.has_in_scope("h3")
                && !self.stack_of_open_elements.has_in_scope("h4")
                && !self.stack_of_open_elements.has_in_scope("h5")
                && !self.stack_of_open_elements.has_in_scope("h6")
            {
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            self.stack_of_open_elements
                .pop_until_one_of_tag_names_popped(&["h1", "h2", "h3", "h4", "h5", "h6"]);
            return;
        }

        // -> A start tag whose tag name is "a"
        if token.is_start_tag() && token.tag_name() == "a" {
            // If the list of active formatting elements contains an a element between the end of
            // the list and the last marker on the list (or the start of the list if there is no marker),
            // then this is a parse error.
            if let Some((_, handle)) = self
                .list_of_active_formatting_elements
                .last_element_with_tag_name_before_marker("a")
            {
                // Run the adoption agency algorithm for the token.
                self.run_the_adoption_agency_algorithm(token);
                // Remove the element from the list and the stack.
                self.list_of_active_formatting_elements.remove(handle);
                self.stack_of_open_elements.remove(handle);
            }
            self.reconstruct_the_active_formatting_elements();
            let element = self.insert_html_element(token);
            self.list_of_active_formatting_elements.add(
                element,
                token.tag_name().to_string(),
                token.clone(),
            );
            return;
        }

        // -> A start tag whose tag name is one of: "b", "big", "code", "em", "font", "i", "s",
        //    "small", "strike", "strong", "tt", "u"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "b" | "big"
                    | "code"
                    | "em"
                    | "font"
                    | "i"
                    | "s"
                    | "small"
                    | "strike"
                    | "strong"
                    | "tt"
                    | "u"
            )
        {
            self.reconstruct_the_active_formatting_elements();
            let element = self.insert_html_element(token);
            self.list_of_active_formatting_elements.add(
                element,
                token.tag_name().to_string(),
                token.clone(),
            );
            return;
        }

        // -> A start tag whose tag name is "nobr"
        if token.is_start_tag() && token.tag_name() == "nobr" {
            self.reconstruct_the_active_formatting_elements();
            if self.stack_of_open_elements.has_in_scope("nobr") {
                // Parse error.
                self.run_the_adoption_agency_algorithm(token);
                self.reconstruct_the_active_formatting_elements();
            }
            let element = self.insert_html_element(token);
            self.list_of_active_formatting_elements.add(
                element,
                "nobr".to_string(),
                token.clone(),
            );
            return;
        }

        // -> An end tag whose tag name is one of: "a", "b", "big", "code", "em", "font", "i",
        //    "nobr", "s", "small", "strike", "strong", "tt", "u"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "a" | "b"
                    | "big"
                    | "code"
                    | "em"
                    | "font"
                    | "i"
                    | "nobr"
                    | "s"
                    | "small"
                    | "strike"
                    | "strong"
                    | "tt"
                    | "u"
            )
        {
            self.run_the_adoption_agency_algorithm(token);
            return;
        }

        // -> A start tag whose tag name is one of: "applet", "marquee", "object"
        if token.is_start_tag()
            && matches!(token.tag_name(), "applet" | "marquee" | "object")
        {
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            self.list_of_active_formatting_elements.add_marker();
            self.frameset_ok = false;
            return;
        }

        // -> An end tag whose tag name is one of: "applet", "marquee", "object"
        if token.is_end_tag()
            && matches!(token.tag_name(), "applet" | "marquee" | "object")
        {
            if !self.stack_of_open_elements.has_in_scope(token.tag_name()) {
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            self.stack_of_open_elements
                .pop_until_tag_name_popped(token.tag_name());
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();
            return;
        }

        // -> A start tag whose tag name is "table"
        if token.is_start_tag() && token.tag_name() == "table" {
            // If the Document is not set to quirks mode, and the stack of open elements
            // has a p element in button scope, then close a p element.
            // FIXME: Check quirks mode — in quirks mode this step is skipped.
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            self.insert_html_element(token);
            self.frameset_ok = false;
            self.insertion_mode = InsertionMode::InTable;
            return;
        }

        // -> An end tag whose tag name is "br"
        // (Parse error. Drop attributes, treat as start tag.)
        if token.is_end_tag() && token.tag_name() == "br" {
            self.reconstruct_the_active_formatting_elements();
            let br_token = Self::make_start_tag_token("br");
            self.insert_html_element(&br_token);
            self.stack_of_open_elements.pop();
            self.frameset_ok = false;
            return;
        }

        // -> A start tag whose tag name is one of: "area", "br", "embed", "img", "keygen", "wbr"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "area" | "br" | "embed" | "img" | "keygen" | "wbr"
            )
        {
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            self.frameset_ok = false;
            return;
        }

        // -> A start tag whose tag name is "input"
        if token.is_start_tag() && token.tag_name() == "input" {
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            // If the token does not have an attribute with the name "type", or if it does,
            // but that attribute's value is not an ASCII case-insensitive match for "hidden",
            // then set the frameset-ok flag to "not ok".
            let is_hidden = token
                .get_attribute("type")
                .is_some_and(|v| v.eq_ignore_ascii_case("hidden"));
            if !is_hidden {
                self.frameset_ok = false;
            }
            return;
        }

        // -> A start tag whose tag name is one of: "param", "source", "track"
        if token.is_start_tag()
            && matches!(token.tag_name(), "param" | "source" | "track")
        {
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            return;
        }

        // -> A start tag whose tag name is "hr"
        if token.is_start_tag() && token.tag_name() == "hr" {
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            self.frameset_ok = false;
            return;
        }

        // -> A start tag whose tag name is "image"
        if token.is_start_tag() && token.tag_name() == "image" {
            // Parse error. Change the token's tag name to "img" and reprocess it.
            // (We create a new token with the corrected name.)
            let mut corrected = token.clone();
            *corrected.tag_name_mut() = "img".to_string();
            self.process_using_the_rules_for(InsertionMode::InBody, &corrected);
            return;
        }

        // -> A start tag whose tag name is "textarea"
        if token.is_start_tag() && token.tag_name() == "textarea" {
            self.insert_html_element(token);
            self.next_line_feed_can_be_ignored = true;
            self.tokenizer.switch_to(TokenizerState::RCDATA);
            self.original_insertion_mode = self.insertion_mode;
            self.frameset_ok = false;
            self.insertion_mode = InsertionMode::Text;
            return;
        }

        // -> A start tag whose tag name is "xmp"
        if token.is_start_tag() && token.tag_name() == "xmp" {
            if self.stack_of_open_elements.has_in_button_scope("p") {
                self.close_a_p_element();
            }
            self.reconstruct_the_active_formatting_elements();
            self.frameset_ok = false;
            self.parse_generic_raw_text_element(token);
            return;
        }

        // -> A start tag whose tag name is "iframe"
        if token.is_start_tag() && token.tag_name() == "iframe" {
            self.frameset_ok = false;
            self.parse_generic_raw_text_element(token);
            return;
        }

        // -> A start tag whose tag name is "noembed"
        // -> A start tag whose tag name is "noscript", if the scripting flag is enabled
        if token.is_start_tag()
            && (token.tag_name() == "noembed"
                || (token.tag_name() == "noscript" && self.scripting_enabled))
        {
            self.parse_generic_raw_text_element(token);
            return;
        }

        // -> A start tag whose tag name is "select"
        if token.is_start_tag() && token.tag_name() == "select" {
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            self.frameset_ok = false;
            // NOTE: The current HTML spec does not have InSelect/InSelectInTable insertion modes.
            // Select elements and their children (option, optgroup) are handled in InBody.
            return;
        }

        // -> A start tag whose tag name is one of: "optgroup", "option"
        if token.is_start_tag() && matches!(token.tag_name(), "optgroup" | "option") {
            if let Some(current) = self.stack_of_open_elements.current_node() {
                if current.is_html_element("option") {
                    self.stack_of_open_elements.pop();
                }
            }
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is one of: "rb", "rtc"
        if token.is_start_tag() && matches!(token.tag_name(), "rb" | "rtc") {
            if self.stack_of_open_elements.has_in_scope("ruby") {
                self.stack_of_open_elements
                    .generate_implied_end_tags(None);
            }
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is one of: "rp", "rt"
        if token.is_start_tag() && matches!(token.tag_name(), "rp" | "rt") {
            if self.stack_of_open_elements.has_in_scope("ruby") {
                self.stack_of_open_elements
                    .generate_implied_end_tags(Some("rtc"));
            }
            self.insert_html_element(token);
            return;
        }

        // -> A start tag whose tag name is "math"
        if token.is_start_tag() && token.tag_name() == "math" {
            self.reconstruct_the_active_formatting_elements();
            // NOTE: MathML and foreign attribute adjustment is done in insert_foreign_element.
            self.insert_foreign_element(token, DomNamespace::MathML, false);
            if token.is_self_closing() {
                self.stack_of_open_elements.pop();
            }
            return;
        }

        // -> A start tag whose tag name is "svg"
        if token.is_start_tag() && token.tag_name() == "svg" {
            self.reconstruct_the_active_formatting_elements();
            // NOTE: SVG tag/attribute and foreign attribute adjustment is done in insert_foreign_element.
            self.insert_foreign_element(token, DomNamespace::SVG, false);
            if token.is_self_closing() {
                self.stack_of_open_elements.pop();
            }
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "col", "colgroup", "frame", "head",
        //    "tbody", "td", "tfoot", "th", "thead", "tr"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption"
                    | "col"
                    | "colgroup"
                    | "frame"
                    | "head"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> Any other start tag
        if token.is_start_tag() {
            self.reconstruct_the_active_formatting_elements();
            self.insert_html_element(token);
            return;
        }

        // -> Any other end tag
        if token.is_end_tag() {
            self.handle_any_other_end_tag_in_body(token);
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inbody
    /// "Any other end tag" handler.
    fn handle_any_other_end_tag_in_body(&mut self, token: &Token) {
        // 1. Initialize node to be the current node (the bottommost node of the stack).
        for i in (0..self.stack_of_open_elements.len()).rev() {
            let entry = self.stack_of_open_elements.entry_at(i);

            // 2. Loop: If node is an HTML element with the same tag name as the token, then:
            if entry.namespace == DomNamespace::HTML && entry.tag_name == token.tag_name() {
                // Generate implied end tags, except for HTML elements with the same tag name
                // as the token.
                self.stack_of_open_elements
                    .generate_implied_end_tags(Some(token.tag_name()));

                // Pop all the nodes from the current node up to node, including node, then stop
                // these steps.
                let handle = self.stack_of_open_elements.entry_at(i).handle;
                while self.stack_of_open_elements.len() > 0 {
                    let popped = self.stack_of_open_elements.pop();
                    if let Some(p) = popped {
                        if p.handle == handle {
                            break;
                        }
                    }
                }
                return;
            }

            // 3. Otherwise, if node is in the special category, then this is a parse error;
            //    ignore the token, and return.
            if is_special_tag(&entry.tag_name, entry.namespace) {
                return;
            }
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#adoption-agency-algorithm
    fn run_the_adoption_agency_algorithm(&mut self, token: &Token) {
        let subject = token.tag_name();

        // 1. If the current node is an HTML element whose tag name is subject, and the current
        //    node is not in the list of active formatting elements, then pop the current node off
        //    the stack of open elements, and return.
        if let Some(current) = self.stack_of_open_elements.current_node() {
            if current.namespace == DomNamespace::HTML
                && current.tag_name == subject
                && !self
                    .list_of_active_formatting_elements
                    .contains(current.handle)
            {
                self.stack_of_open_elements.pop();
                return;
            }
        }

        // 2. Let outer loop counter be 0.
        // 3. While true:
        for _outer_loop_counter in 0..8 {

            // 6. Let formatting element be the last element in the list of active formatting
            //    elements that: is between the end of the list and the last marker in the list,
            //    if any, or the start of the list otherwise; and has the tag name subject.
            let formatting_info = self
                .list_of_active_formatting_elements
                .last_element_with_tag_name_before_marker(subject);

            // If there is no such element, then return and instead act as described in the
            // "any other end tag" entry above.
            let (_formatting_list_index, formatting_element_handle) = match formatting_info {
                Some(info) => info,
                None => {
                    self.handle_any_other_end_tag_in_body(token);
                    return;
                }
            };

            // 7. If formatting element is not in the stack of open elements, then this is a
            //    parse error; remove the element from the list, and return.
            let formatting_stack_index =
                match self.stack_of_open_elements.index_of(formatting_element_handle) {
                    Some(idx) => idx,
                    None => {
                        self.list_of_active_formatting_elements
                            .remove(formatting_element_handle);
                        return;
                    }
                };

            // 8. If formatting element is in the stack of open elements, but the element is
            //    not in scope, then this is a parse error; return.
            if !self
                .stack_of_open_elements
                .has_element_in_scope(formatting_element_handle)
            {
                return;
            }

            // 9. If formatting element is not the current node, this is a parse error.
            // (Just log it.)

            // 10. Let furthest block be the topmost node in the stack of open elements that is
            //     lower in the stack than formatting element, and is an element in the special category.
            let mut furthest_block_index = None;
            for i in (formatting_stack_index + 1)..self.stack_of_open_elements.len() {
                let entry = self.stack_of_open_elements.entry_at(i);
                if is_special_tag(&entry.tag_name, entry.namespace) {
                    furthest_block_index = Some(i);
                    break;
                }
            }

            // 11. If there is no furthest block, then the UA must first pop all the nodes from
            //     the bottom of the stack of open elements, from the current node up to and
            //     including formatting element, then remove formatting element from the list,
            //     and return.
            if furthest_block_index.is_none() {
                while self.stack_of_open_elements.len() > formatting_stack_index {
                    self.stack_of_open_elements.pop();
                }
                self.list_of_active_formatting_elements
                    .remove(formatting_element_handle);
                return;
            }

            let furthest_block_index = furthest_block_index.unwrap();

            // 12. Let commonAncestor be the element immediately above formattingElement in the stack.
            let common_ancestor = self
                .stack_of_open_elements
                .element_immediately_above(formatting_element_handle)
                .map(|e| e.handle);

            // 13. Let a bookmark note the position of formattingElement in the list.
            let mut bookmark = self
                .list_of_active_formatting_elements
                .index_of(formatting_element_handle)
                .unwrap();

            // 14. Let node and lastNode be furthestBlock.
            let furthest_block_handle = self
                .stack_of_open_elements
                .entry_at(furthest_block_index)
                .handle;
            let mut node_handle = furthest_block_handle;
            let mut last_node_handle = furthest_block_handle;

            let mut node_above_node = self
                .stack_of_open_elements
                .element_immediately_above(node_handle)
                .map(|e| e.handle);

            // 15. Let innerLoopCounter be 0.
            // 16. While true:
            for _inner_loop_counter in 1..=usize::MAX {
                // 1. Let node be the element immediately above node in the stack.
                node_handle = match node_above_node {
                    Some(h) => h,
                    None => break,
                };

                node_above_node = self
                    .stack_of_open_elements
                    .element_immediately_above(node_handle)
                    .map(|e| e.handle);

                // 2. If node is formattingElement, then break.
                if node_handle == formatting_element_handle {
                    break;
                }

                // 3. If innerLoopCounter > 3 and node is in the formatting list, remove it.
                let node_formatting_index = self
                    .list_of_active_formatting_elements
                    .index_of(node_handle);
                if _inner_loop_counter > 3 && node_formatting_index.is_some() {
                    let idx = node_formatting_index.unwrap();
                    if idx < bookmark {
                        bookmark -= 1;
                    }
                    self.list_of_active_formatting_elements.remove(node_handle);
                }

                // 4. If node is not in the formatting list, remove from stack and continue.
                let node_formatting_index = self
                    .list_of_active_formatting_elements
                    .index_of(node_handle);
                if node_formatting_index.is_none() {
                    self.stack_of_open_elements.remove(node_handle);
                    continue;
                }

                // 5. Create an element for the token, replace in both lists.
                let formatting_idx = node_formatting_index.unwrap();
                let token_clone = {
                    let entry = &self.list_of_active_formatting_elements.entries()[formatting_idx];
                    entry.token().unwrap().clone()
                };
                let tag_name = token_clone.tag_name().to_string();
                let new_element = self.create_element_for(&token_clone, DomNamespace::HTML);
                for attr in token_clone.attributes() {
                    new_element.append_attribute(&attr.local_name, &attr.value);
                }

                self.list_of_active_formatting_elements.replace(
                    node_handle,
                    new_element,
                    tag_name.clone(),
                    token_clone.clone(),
                );
                self.stack_of_open_elements.replace(
                    node_handle,
                    StackEntry::new(new_element, tag_name, DomNamespace::HTML),
                );
                node_handle = new_element;

                // 6. If lastNode is furthestBlock, move bookmark.
                if last_node_handle == furthest_block_handle {
                    bookmark = self
                        .list_of_active_formatting_elements
                        .index_of(node_handle)
                        .unwrap()
                        + 1;
                }

                // 7. Append lastNode to node.
                DomHandle::insert_before(node_handle, last_node_handle, None);

                // 8. Set lastNode to node.
                last_node_handle = node_handle;
            }

            // 17. Insert lastNode at the appropriate place, using commonAncestor as override target.
            if let Some(ancestor) = common_ancestor {
                let adjusted = self.find_appropriate_place_for_inserting_node(Some(ancestor));
                self.insert_at_adjusted_location(last_node_handle, &adjusted);
            }

            // 18. Create an element for the token for which formattingElement was created.
            let formatting_idx = self
                .list_of_active_formatting_elements
                .index_of(formatting_element_handle);
            let (token_clone, tag_name) = if let Some(idx) = formatting_idx {
                let entry = &self.list_of_active_formatting_elements.entries()[idx];
                (
                    entry.token().unwrap().clone(),
                    entry.tag_name().unwrap().to_string(),
                )
            } else {
                return;
            };

            let new_element = self.create_element_for(&token_clone, DomNamespace::HTML);
            for attr in token_clone.attributes() {
                new_element.append_attribute(&attr.local_name, &attr.value);
            }

            // 19. Take all child nodes of furthestBlock and append them to the new element.
            unsafe {
                crate::dom_bridge::html_parser_bridge_reparent_children(
                    furthest_block_handle.as_ptr(),
                    new_element.as_ptr(),
                );
            }

            // 20. Append the new element to furthestBlock.
            DomHandle::insert_before(furthest_block_handle, new_element, None);

            // 21. Remove formattingElement from the list, insert new element at bookmark.
            let formatting_idx = self
                .list_of_active_formatting_elements
                .index_of(formatting_element_handle);
            if let Some(idx) = formatting_idx {
                if idx < bookmark {
                    bookmark -= 1;
                }
            }
            self.list_of_active_formatting_elements
                .remove(formatting_element_handle);
            self.list_of_active_formatting_elements.insert_at(
                bookmark,
                new_element,
                tag_name.clone(),
                token_clone,
            );

            // 22. Remove formattingElement from the stack, insert new element below furthestBlock.
            self.stack_of_open_elements
                .remove(formatting_element_handle);
            self.stack_of_open_elements.insert_immediately_below(
                StackEntry::new(new_element, tag_name, DomNamespace::HTML),
                furthest_block_handle,
            );
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-incdata
    fn handle_text(&mut self, token: &Token) {
        // -> A character token
        if token.is_character() {
            // Insert the character.
            self.insert_character(token.code_point);
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            // Parse error.
            // If the current node is a script element, then set its already started to true.
            // Pop the current node off the stack of open elements.
            self.stack_of_open_elements.pop();
            // Switch the insertion mode to the original insertion mode and reprocess the token.
            self.insertion_mode = self.original_insertion_mode;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is "script"
        if token.is_end_tag() && token.tag_name() == "script" {
            // Pop the current node off the stack (which will be the script element).
            let script_entry = self.stack_of_open_elements.pop();

            // Switch the insertion mode to the original insertion mode.
            self.insertion_mode = self.original_insertion_mode;

            // Let the old insertion point have the same value as the current insertion point.
            self.tokenizer.store_insertion_point();
            // Let the insertion point be just before the next input character.
            self.tokenizer.update_insertion_point();

            // Prepare and execute the script element.
            if let Some(entry) = script_entry {
                self.flush_character_insertions();
                entry.handle.execute_script(self.document);
            }

            // Let the insertion point have the value of the old insertion point.
            self.tokenizer.restore_insertion_point();
            return;
        }

        // -> Any other end tag
        if token.is_end_tag() {
            // Pop the current node off the stack of open elements.
            self.stack_of_open_elements.pop();
            // Switch the insertion mode to the original insertion mode.
            self.insertion_mode = self.original_insertion_mode;
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-afterbody
    fn handle_after_body(&mut self, token: &Token) {
        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        if token.is_parser_whitespace() {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A comment token
        if token.is_comment() {
            // Insert a comment as the last child of the first element in the stack of open
            // elements (the html element).
            if let Some(html) = self.stack_of_open_elements.first() {
                let comment = DomHandle::create_comment(self.document, token.comment_data());
                DomHandle::insert_before(html.handle, comment, None);
            }
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> An end tag whose tag name is "html"
        if token.is_end_tag() && token.tag_name() == "html" {
            // If the parser was created as part of the HTML fragment parsing algorithm, this is
            // a parse error; ignore the token. (fragment case)
            if self.parsing_fragment {
                return;
            }
            // Otherwise, switch the insertion mode to "after after body".
            self.insertion_mode = InsertionMode::AfterAfterBody;
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            // Stop parsing.
            self.stop_parsing();
            return;
        }

        // -> Anything else
        // Parse error. Switch the insertion mode to "in body" and reprocess the token.
        self.insertion_mode = InsertionMode::InBody;
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-after-after-body-insertion-mode
    fn handle_after_after_body(&mut self, token: &Token) {
        // -> A comment token
        if token.is_comment() {
            // Insert a comment as the last child of the Document object.
            let comment = DomHandle::create_comment(self.document, token.comment_data());
            let doc_node = DomHandle::document_node(self.document);
            DomHandle::insert_before(doc_node, comment, None);
            return;
        }

        // -> A DOCTYPE token
        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        // -> A start tag whose tag name is "html"
        if token.is_doctype()
            || token.is_parser_whitespace()
            || (token.is_start_tag() && token.tag_name() == "html")
        {
            // Process the token using the rules for the "in body" insertion mode.
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            // Stop parsing.
            self.stop_parsing();
            return;
        }

        // -> Anything else
        // Parse error. Switch the insertion mode to "in body" and reprocess the token.
        self.insertion_mode = InsertionMode::InBody;
        self.reprocess_token();
    }

    // =======================================================================
    // Stub handlers for modes not yet fully implemented
    // =======================================================================

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intable
    fn handle_in_table(&mut self, token: &Token) {
        // -> A character token, if the current node is table, tbody, template, tfoot, thead, or tr element
        if token.is_character() {
            if let Some(current) = self.stack_of_open_elements.current_node() {
                if current.namespace == DomNamespace::HTML
                    && matches!(
                        current.tag_name.as_str(),
                        "table" | "tbody" | "template" | "tfoot" | "thead" | "tr"
                    )
                {
                    // Let the pending table character tokens be an empty list of tokens.
                    self.pending_table_character_tokens.clear();
                    // Set the original insertion mode to the current insertion mode.
                    self.original_insertion_mode = self.insertion_mode;
                    // Switch the insertion mode to "in table text" and reprocess the token.
                    self.insertion_mode = InsertionMode::InTableText;
                    self.reprocess_token();
                    return;
                }
            }
        }

        // -> A comment token
        if token.is_comment() {
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "caption"
        if token.is_start_tag() && token.tag_name() == "caption" {
            // Clear the stack back to a table context.
            self.stack_of_open_elements.clear_back_to_table_context();
            // Insert a marker at the end of the list of active formatting elements.
            self.list_of_active_formatting_elements.add_marker();
            // Insert an HTML element for the token, then switch the insertion mode to "in caption".
            self.insert_html_element(token);
            self.insertion_mode = InsertionMode::InCaption;
            return;
        }

        // -> A start tag whose tag name is "colgroup"
        if token.is_start_tag() && token.tag_name() == "colgroup" {
            self.stack_of_open_elements.clear_back_to_table_context();
            self.insert_html_element(token);
            self.insertion_mode = InsertionMode::InColumnGroup;
            return;
        }

        // -> A start tag whose tag name is "col"
        if token.is_start_tag() && token.tag_name() == "col" {
            self.stack_of_open_elements.clear_back_to_table_context();
            let colgroup_token = Self::make_start_tag_token("colgroup");
            self.insert_html_element(&colgroup_token);
            self.insertion_mode = InsertionMode::InColumnGroup;
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is one of: "tbody", "tfoot", "thead"
        if token.is_start_tag()
            && matches!(token.tag_name(), "tbody" | "tfoot" | "thead")
        {
            self.stack_of_open_elements.clear_back_to_table_context();
            self.insert_html_element(token);
            self.insertion_mode = InsertionMode::InTableBody;
            return;
        }

        // -> A start tag whose tag name is one of: "td", "th", "tr"
        if token.is_start_tag()
            && matches!(token.tag_name(), "td" | "th" | "tr")
        {
            self.stack_of_open_elements.clear_back_to_table_context();
            let tbody_token = Self::make_start_tag_token("tbody");
            self.insert_html_element(&tbody_token);
            self.insertion_mode = InsertionMode::InTableBody;
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is "table"
        if token.is_start_tag() && token.tag_name() == "table" {
            // Parse error.
            if !self.stack_of_open_elements.has_in_table_scope("table") {
                return;
            }
            self.stack_of_open_elements
                .pop_until_tag_name_popped("table");
            self.reset_the_insertion_mode_appropriately();
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is "table"
        if token.is_end_tag() && token.tag_name() == "table" {
            if !self.stack_of_open_elements.has_in_table_scope("table") {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .pop_until_tag_name_popped("table");
            self.reset_the_insertion_mode_appropriately();
            return;
        }

        // -> An end tag whose tag name is one of: "body", "caption", "col", "colgroup", "html",
        //    "tbody", "td", "tfoot", "th", "thead", "tr"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "body"
                    | "caption"
                    | "col"
                    | "colgroup"
                    | "html"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is one of: "style", "script", "template"
        // -> An end tag whose tag name is "template"
        if (token.is_start_tag()
            && matches!(token.tag_name(), "style" | "script" | "template"))
            || (token.is_end_tag() && token.tag_name() == "template")
        {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> A start tag whose tag name is "input"
        if token.is_start_tag() && token.tag_name() == "input" {
            let is_hidden = token
                .get_attribute("type")
                .is_some_and(|v| v.eq_ignore_ascii_case("hidden"));
            if !is_hidden {
                // Act as described in the "anything else" entry below.
                self.foster_parenting = true;
                self.process_using_the_rules_for(InsertionMode::InBody, token);
                self.foster_parenting = false;
                return;
            }
            // Parse error. Insert an HTML element for the token. Pop it off.
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            return;
        }

        // -> A start tag whose tag name is "form"
        if token.is_start_tag() && token.tag_name() == "form" {
            // Parse error.
            if self.form_element.is_some()
                || self.stack_of_open_elements.contains_template_element()
            {
                return;
            }
            let element = self.insert_html_element(token);
            self.form_element = Some(element);
            self.stack_of_open_elements.pop();
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> Anything else
        // Parse error. Enable foster parenting, process using InBody, disable foster parenting.
        self.foster_parenting = true;
        self.process_using_the_rules_for(InsertionMode::InBody, token);
        self.foster_parenting = false;
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intabletext
    fn handle_in_table_text(&mut self, token: &Token) {
        if token.is_character() {
            // -> A character token that is U+0000 NULL
            if token.code_point == 0 {
                // Parse error. Ignore the token.
                return;
            }
            // -> Any other character token
            // Append the character token to the pending table character tokens list.
            self.pending_table_character_tokens.push(token.clone());
            return;
        }

        // -> Anything else
        // If any of the tokens in the pending table character tokens list are character tokens
        // that are not ASCII whitespace, then this is a parse error: reprocess them using InBody
        // with foster parenting enabled.
        let has_non_whitespace = self
            .pending_table_character_tokens
            .iter()
            .any(|t| !t.is_parser_whitespace());

        let pending = std::mem::take(&mut self.pending_table_character_tokens);
        if has_non_whitespace {
            for pending_token in &pending {
                self.foster_parenting = true;
                self.process_using_the_rules_for(InsertionMode::InBody, pending_token);
                self.foster_parenting = false;
            }
        } else {
            // Otherwise, insert the characters.
            for pending_token in &pending {
                self.insert_character(pending_token.code_point);
            }
        }

        // Switch the insertion mode to the original insertion mode and reprocess the token.
        self.insertion_mode = self.original_insertion_mode;
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-incaption
    fn handle_in_caption(&mut self, token: &Token) {
        // -> An end tag whose tag name is "caption"
        if token.is_end_tag() && token.tag_name() == "caption" {
            if !self.stack_of_open_elements.has_in_table_scope("caption") {
                // Parse error. Ignore the token. (fragment case)
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            self.stack_of_open_elements
                .pop_until_tag_name_popped("caption");
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();
            self.insertion_mode = InsertionMode::InTable;
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "col", "colgroup", "tbody", "td",
        //    "tfoot", "th", "thead", "tr"
        // -> An end tag whose tag name is "table"
        if (token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption"
                    | "col"
                    | "colgroup"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
            ))
            || (token.is_end_tag() && token.tag_name() == "table")
        {
            if !self.stack_of_open_elements.has_in_table_scope("caption") {
                // Parse error. Ignore the token. (fragment case)
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            self.stack_of_open_elements
                .pop_until_tag_name_popped("caption");
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();
            self.insertion_mode = InsertionMode::InTable;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "body", "col", "colgroup", "html", "tbody",
        //    "td", "tfoot", "th", "thead", "tr"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "body"
                    | "col"
                    | "colgroup"
                    | "html"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        self.process_using_the_rules_for(InsertionMode::InBody, token);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-incolgroup
    fn handle_in_column_group(&mut self, token: &Token) {
        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        if token.is_parser_whitespace() {
            self.insert_character(token.code_point);
            return;
        }

        // -> A comment token
        if token.is_comment() {
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is "col"
        if token.is_start_tag() && token.tag_name() == "col" {
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            return;
        }

        // -> An end tag whose tag name is "colgroup"
        if token.is_end_tag() && token.tag_name() == "colgroup" {
            if let Some(current) = self.stack_of_open_elements.current_node() {
                if !(current.namespace == DomNamespace::HTML && current.tag_name == "colgroup") {
                    // Parse error. Ignore the token.
                    return;
                }
            }
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTable;
            return;
        }

        // -> An end tag whose tag name is "col"
        if token.is_end_tag() && token.tag_name() == "col" {
            // Parse error. Ignore the token.
            return;
        }

        // -> A start tag whose tag name is "template"
        // -> An end tag whose tag name is "template"
        if (token.is_start_tag() || token.is_end_tag()) && token.tag_name() == "template" {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> Anything else
        if let Some(current) = self.stack_of_open_elements.current_node() {
            if !(current.namespace == DomNamespace::HTML && current.tag_name == "colgroup") {
                // Parse error. Ignore the token.
                return;
            }
        }
        self.stack_of_open_elements.pop();
        self.insertion_mode = InsertionMode::InTable;
        self.reprocess_token();
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intbody
    fn handle_in_table_body(&mut self, token: &Token) {
        // -> A start tag whose tag name is "tr"
        if token.is_start_tag() && token.tag_name() == "tr" {
            self.stack_of_open_elements
                .clear_back_to_table_body_context();
            self.insert_html_element(token);
            self.insertion_mode = InsertionMode::InRow;
            return;
        }

        // -> A start tag whose tag name is one of: "th", "td"
        if token.is_start_tag() && matches!(token.tag_name(), "th" | "td") {
            // Parse error.
            self.stack_of_open_elements
                .clear_back_to_table_body_context();
            let tr_token = Self::make_start_tag_token("tr");
            self.insert_html_element(&tr_token);
            self.insertion_mode = InsertionMode::InRow;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "tbody", "tfoot", "thead"
        if token.is_end_tag()
            && matches!(token.tag_name(), "tbody" | "tfoot" | "thead")
        {
            if !self
                .stack_of_open_elements
                .has_in_table_scope(token.tag_name())
            {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .clear_back_to_table_body_context();
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTable;
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "col", "colgroup", "tbody",
        //    "tfoot", "thead"
        // -> An end tag whose tag name is "table"
        if (token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
            ))
            || (token.is_end_tag() && token.tag_name() == "table")
        {
            if !self.stack_of_open_elements.has_in_table_scope("tbody")
                && !self.stack_of_open_elements.has_in_table_scope("thead")
                && !self.stack_of_open_elements.has_in_table_scope("tfoot")
            {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .clear_back_to_table_body_context();
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTable;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "body", "caption", "col", "colgroup", "html",
        //    "td", "th", "tr"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        self.process_using_the_rules_for(InsertionMode::InTable, token);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intr
    fn handle_in_row(&mut self, token: &Token) {
        // -> A start tag whose tag name is one of: "th", "td"
        if token.is_start_tag() && matches!(token.tag_name(), "th" | "td") {
            self.stack_of_open_elements
                .clear_back_to_table_row_context();
            self.insert_html_element(token);
            self.insertion_mode = InsertionMode::InCell;
            self.list_of_active_formatting_elements.add_marker();
            return;
        }

        // -> An end tag whose tag name is "tr"
        if token.is_end_tag() && token.tag_name() == "tr" {
            if !self.stack_of_open_elements.has_in_table_scope("tr") {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .clear_back_to_table_row_context();
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTableBody;
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "col", "colgroup", "tbody",
        //    "tfoot", "thead", "tr"
        // -> An end tag whose tag name is "table"
        if (token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
            ))
            || (token.is_end_tag() && token.tag_name() == "table")
        {
            if !self.stack_of_open_elements.has_in_table_scope("tr") {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .clear_back_to_table_row_context();
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTableBody;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "tbody", "tfoot", "thead"
        if token.is_end_tag()
            && matches!(token.tag_name(), "tbody" | "tfoot" | "thead")
        {
            if !self
                .stack_of_open_elements
                .has_in_table_scope(token.tag_name())
            {
                // Parse error. Ignore the token.
                return;
            }
            if !self.stack_of_open_elements.has_in_table_scope("tr") {
                return;
            }
            self.stack_of_open_elements
                .clear_back_to_table_row_context();
            self.stack_of_open_elements.pop();
            self.insertion_mode = InsertionMode::InTableBody;
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "body", "caption", "col", "colgroup",
        //    "html", "td", "th"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> Anything else
        self.process_using_the_rules_for(InsertionMode::InTable, token);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#close-the-cell
    fn close_the_cell(&mut self) {
        self.stack_of_open_elements
            .generate_implied_end_tags(None);
        // Pop elements until a td or th element has been popped.
        self.stack_of_open_elements
            .pop_until_one_of_tag_names_popped(&["td", "th"]);
        self.list_of_active_formatting_elements
            .clear_up_to_last_marker();
        self.insertion_mode = InsertionMode::InRow;
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intd
    fn handle_in_cell(&mut self, token: &Token) {
        // -> An end tag whose tag name is one of: "td", "th"
        if token.is_end_tag() && matches!(token.tag_name(), "td" | "th") {
            if !self
                .stack_of_open_elements
                .has_in_table_scope(token.tag_name())
            {
                // Parse error. Ignore the token.
                return;
            }
            self.stack_of_open_elements
                .generate_implied_end_tags(None);
            self.stack_of_open_elements
                .pop_until_tag_name_popped(token.tag_name());
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();
            self.insertion_mode = InsertionMode::InRow;
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "col", "colgroup", "tbody", "td",
        //    "tfoot", "th", "thead", "tr"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption"
                    | "col"
                    | "colgroup"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
            )
        {
            if !self.stack_of_open_elements.has_in_table_scope("td")
                && !self.stack_of_open_elements.has_in_table_scope("th")
            {
                // Parse error. Ignore the token. (fragment case)
                return;
            }
            self.close_the_cell();
            self.reprocess_token();
            return;
        }

        // -> An end tag whose tag name is one of: "body", "caption", "col", "colgroup", "html"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "body" | "caption" | "col" | "colgroup" | "html"
            )
        {
            // Parse error. Ignore the token.
            return;
        }

        // -> An end tag whose tag name is one of: "table", "tbody", "tfoot", "thead", "tr"
        if token.is_end_tag()
            && matches!(
                token.tag_name(),
                "table" | "tbody" | "tfoot" | "thead" | "tr"
            )
        {
            if !self
                .stack_of_open_elements
                .has_in_table_scope(token.tag_name())
            {
                // Parse error. Ignore the token.
                return;
            }
            self.close_the_cell();
            self.reprocess_token();
            return;
        }

        // -> Anything else
        self.process_using_the_rules_for(InsertionMode::InBody, token);
    }

    fn handle_in_select(&mut self, token: &Token) {
        // NOTE: The current HTML spec no longer has InSelect mode.
        // This is kept as a fallback that processes tokens using InBody rules.
        self.process_using_the_rules_for(InsertionMode::InBody, token);
    }

    fn handle_in_select_in_table(&mut self, token: &Token) {
        // NOTE: The current HTML spec no longer has InSelectInTable mode.
        self.process_using_the_rules_for(InsertionMode::InBody, token);
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-intemplate
    fn handle_in_template(&mut self, token: &Token) {
        // -> A character token, A comment token, A DOCTYPE token
        if token.is_character() || token.is_comment() || token.is_doctype() {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is one of: "base", "basefont", "bgsound", "link", "meta",
        //    "noframes", "script", "style", "template", "title"
        // -> An end tag whose tag name is "template"
        if (token.is_start_tag()
            && matches!(
                token.tag_name(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            ))
            || (token.is_end_tag() && token.tag_name() == "template")
        {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> A start tag whose tag name is one of: "caption", "colgroup", "tbody", "tfoot", "thead"
        if token.is_start_tag()
            && matches!(
                token.tag_name(),
                "caption" | "colgroup" | "tbody" | "tfoot" | "thead"
            )
        {
            self.stack_of_template_insertion_modes.pop();
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InTable);
            self.insertion_mode = InsertionMode::InTable;
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is "col"
        if token.is_start_tag() && token.tag_name() == "col" {
            self.stack_of_template_insertion_modes.pop();
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InColumnGroup);
            self.insertion_mode = InsertionMode::InColumnGroup;
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is "tr"
        if token.is_start_tag() && token.tag_name() == "tr" {
            self.stack_of_template_insertion_modes.pop();
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InTableBody);
            self.insertion_mode = InsertionMode::InTableBody;
            self.reprocess_token();
            return;
        }

        // -> A start tag whose tag name is one of: "td", "th"
        if token.is_start_tag() && matches!(token.tag_name(), "td" | "th") {
            self.stack_of_template_insertion_modes.pop();
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InRow);
            self.insertion_mode = InsertionMode::InRow;
            self.reprocess_token();
            return;
        }

        // -> Any other start tag
        if token.is_start_tag() {
            self.stack_of_template_insertion_modes.pop();
            self.stack_of_template_insertion_modes
                .push(InsertionMode::InBody);
            self.insertion_mode = InsertionMode::InBody;
            self.reprocess_token();
            return;
        }

        // -> Any other end tag
        if token.is_end_tag() {
            // Parse error. Ignore the token.
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            if !self.stack_of_open_elements.contains_template_element() {
                self.stop_parsing();
                return;
            }
            // Parse error.
            self.stack_of_open_elements
                .pop_until_tag_name_popped("template");
            self.list_of_active_formatting_elements
                .clear_up_to_last_marker();
            self.stack_of_template_insertion_modes.pop();
            self.reset_the_insertion_mode_appropriately();
            self.reprocess_token();
        }
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inframeset
    fn handle_in_frameset(&mut self, token: &Token) {
        // -> A character token that is one of U+0009, U+000A, U+000C, U+000D, or U+0020
        if token.is_parser_whitespace() {
            self.insert_character(token.code_point);
            return;
        }

        // -> A comment token
        if token.is_comment() {
            self.insert_comment(token);
            return;
        }

        // -> A DOCTYPE token
        if token.is_doctype() {
            return;
        }

        // -> A start tag whose tag name is "html"
        if token.is_start_tag() && token.tag_name() == "html" {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }

        // -> A start tag whose tag name is "frameset"
        if token.is_start_tag() && token.tag_name() == "frameset" {
            self.insert_html_element(token);
            return;
        }

        // -> An end tag whose tag name is "frameset"
        if token.is_end_tag() && token.tag_name() == "frameset" {
            // If the current node is the root html element, parse error; ignore the token.
            if self.stack_of_open_elements.len() == 1 {
                return;
            }
            self.stack_of_open_elements.pop();
            // If not fragment case and current node is no longer a frameset, switch mode.
            if !self.parsing_fragment {
                if let Some(current) = self.stack_of_open_elements.current_node() {
                    if !(current.namespace == DomNamespace::HTML
                        && current.tag_name == "frameset")
                    {
                        self.insertion_mode = InsertionMode::AfterFrameset;
                    }
                }
            }
            return;
        }

        // -> A start tag whose tag name is "frame"
        if token.is_start_tag() && token.tag_name() == "frame" {
            self.insert_html_element(token);
            self.stack_of_open_elements.pop();
            return;
        }

        // -> A start tag whose tag name is "noframes"
        if token.is_start_tag() && token.tag_name() == "noframes" {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }

        // -> An end-of-file token
        if token.is_eof() {
            self.stop_parsing();
            return;
        }

        // -> Anything else: parse error, ignore the token.
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-afterframeset
    fn handle_after_frameset(&mut self, token: &Token) {
        if token.is_parser_whitespace() {
            self.insert_character(token.code_point);
            return;
        }
        if token.is_comment() {
            self.insert_comment(token);
            return;
        }
        if token.is_doctype() {
            return;
        }
        if token.is_start_tag() && token.tag_name() == "html" {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }
        if token.is_end_tag() && token.tag_name() == "html" {
            self.insertion_mode = InsertionMode::AfterAfterFrameset;
            return;
        }
        if token.is_start_tag() && token.tag_name() == "noframes" {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }
        if token.is_eof() {
            self.stop_parsing();
            return;
        }
        // Anything else: parse error, ignore.
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#the-after-after-frameset-insertion-mode
    fn handle_after_after_frameset(&mut self, token: &Token) {
        if token.is_comment() {
            let comment = DomHandle::create_comment(self.document, token.comment_data());
            let doc_node = DomHandle::document_node(self.document);
            DomHandle::insert_before(doc_node, comment, None);
            return;
        }
        if token.is_doctype()
            || token.is_parser_whitespace()
            || (token.is_start_tag() && token.tag_name() == "html")
        {
            self.process_using_the_rules_for(InsertionMode::InBody, token);
            return;
        }
        if token.is_eof() {
            self.stop_parsing();
            return;
        }
        if token.is_start_tag() && token.tag_name() == "noframes" {
            self.process_using_the_rules_for(InsertionMode::InHead, token);
            return;
        }
        // Anything else: parse error, ignore.
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inforeign
    fn process_using_the_rules_for_foreign_content(&mut self, token: &Token) {
        // TODO: Full foreign content implementation
        // For now, handle the basic break-out cases.
        if token.is_start_tag() {
            let tag = token.tag_name();
            // If the token is one of the "break-out" tags, process using HTML rules.
            if matches!(
                tag,
                "b" | "big"
                    | "blockquote"
                    | "body"
                    | "br"
                    | "center"
                    | "code"
                    | "dd"
                    | "div"
                    | "dl"
                    | "dt"
                    | "em"
                    | "embed"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "head"
                    | "hr"
                    | "i"
                    | "img"
                    | "li"
                    | "listing"
                    | "menu"
                    | "meta"
                    | "nobr"
                    | "ol"
                    | "p"
                    | "pre"
                    | "ruby"
                    | "s"
                    | "small"
                    | "span"
                    | "strong"
                    | "strike"
                    | "sub"
                    | "sup"
                    | "table"
                    | "tt"
                    | "u"
                    | "ul"
                    | "var"
            ) || (tag == "font"
                && (token.has_attribute("color")
                    || token.has_attribute("face")
                    || token.has_attribute("size")))
            {
                // Parse error. Pop elements until current node is in HTML namespace.
                while let Some(current) = self.stack_of_open_elements.current_node() {
                    if current.namespace == DomNamespace::HTML {
                        break;
                    }
                    self.stack_of_open_elements.pop();
                }
                // Reprocess the token.
                self.reprocess_token();
                return;
            }
        }

        if token.is_character() {
            if token.code_point == 0 {
                // Replace with U+FFFD.
                self.insert_character(0xFFFD);
            } else {
                self.insert_character(token.code_point);
                if !token.is_parser_whitespace() {
                    self.frameset_ok = false;
                }
            }
            return;
        }

        if token.is_comment() {
            self.insert_comment(token);
            return;
        }

        // For other tokens in foreign content, insert the element in the appropriate namespace.
        if token.is_start_tag() {
            let namespace = self
                .adjusted_current_node()
                .map(|n| n.namespace)
                .unwrap_or(DomNamespace::HTML);

            // https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inforeign
            // If the current node is an element in the SVG namespace, adjust SVG tag names.
            if namespace == DomNamespace::SVG {
                let adjusted_name = adjust_svg_tag_name(token.tag_name());
                if adjusted_name != token.tag_name() {
                    let mut adjusted_token = token.clone();
                    *adjusted_token.tag_name_mut() = adjusted_name.to_string();
                    self.insert_foreign_element(&adjusted_token, namespace, false);
                    if token.is_self_closing() {
                        self.stack_of_open_elements.pop();
                    }
                    return;
                }
            }

            self.insert_foreign_element(token, namespace, false);
            if token.is_self_closing() {
                self.stack_of_open_elements.pop();
            }
            return;
        }

        if token.is_end_tag() {
            // https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inforeign
            // 1. Initialize node to be the current node.
            let tag = token.tag_name();
            let stack_len = self.stack_of_open_elements.len();
            if stack_len == 0 {
                return;
            }

            // Start from the current node (top of stack).
            let mut i = stack_len - 1;

            // 3. Loop:
            loop {
                let entry = self.stack_of_open_elements.entry_at(i);

                // If node is the topmost element in the stack, return. (fragment case)
                if i == 0 {
                    return;
                }

                // 4. If node's tag name matches the token's tag name, pop elements until
                //    node has been popped, then return.
                if entry.tag_name.eq_ignore_ascii_case(tag) {
                    // Pop elements from the current node up to and including node.
                    while self.stack_of_open_elements.len() > i + 1 {
                        self.stack_of_open_elements.pop();
                    }
                    self.stack_of_open_elements.pop(); // pop node itself
                    return;
                }

                // 5. Set node to the previous entry in the stack.
                i -= 1;
                let next_entry = self.stack_of_open_elements.entry_at(i);

                // 6. If node is not in the HTML namespace, return to loop.
                if next_entry.namespace != DomNamespace::HTML {
                    continue;
                }

                // 7. Otherwise, process the token using the current insertion mode rules.
                self.process_using_the_rules_for(self.insertion_mode, token);
                return;
            }
        }
    }

    // =======================================================================
    // GC integration
    // =======================================================================

    /// Visit all DOM handles for garbage collection.
    pub fn visit_dom_handles(&self, visitor: *mut std::ffi::c_void) {
        unsafe {
            crate::dom_bridge::html_parser_bridge_visit_node(visitor, self.document.as_ptr());
        }
        if let Some(head) = self.head_element {
            unsafe {
                crate::dom_bridge::html_parser_bridge_visit_node(visitor, head.as_ptr());
            }
        }
        if let Some(form) = self.form_element {
            unsafe {
                crate::dom_bridge::html_parser_bridge_visit_node(visitor, form.as_ptr());
            }
        }
        if let Some(ctx) = self.context_element {
            unsafe {
                crate::dom_bridge::html_parser_bridge_visit_node(visitor, ctx.as_ptr());
            }
        }
        if let Some(char_node) = self.character_insertion_node {
            unsafe {
                crate::dom_bridge::html_parser_bridge_visit_node(visitor, char_node.as_ptr());
            }
        }
        self.stack_of_open_elements.visit_dom_handles(visitor);
        self.list_of_active_formatting_elements
            .visit_dom_handles(visitor);
    }
}

// =======================================================================
// Quirks mode public ID prefixes
// https://html.spec.whatwg.org/multipage/parsing.html#the-initial-insertion-mode
// =======================================================================

/// https://html.spec.whatwg.org/multipage/parsing.html#adjust-svg-attributes
fn adjust_svg_tag_name(tag_name: &str) -> &str {
    match tag_name {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        _ => tag_name,
    }
}

/// https://html.spec.whatwg.org/multipage/parsing.html#adjust-foreign-attributes
/// Returns (namespace, prefix, local_name) if the attribute needs namespace adjustment.
fn adjust_foreign_attribute(name: &str) -> Option<(DomNamespace, &str, &str)> {
    match name {
        "xlink:actuate" => Some((DomNamespace::XLink, "xlink", "actuate")),
        "xlink:arcrole" => Some((DomNamespace::XLink, "xlink", "arcrole")),
        "xlink:href" => Some((DomNamespace::XLink, "xlink", "href")),
        "xlink:role" => Some((DomNamespace::XLink, "xlink", "role")),
        "xlink:show" => Some((DomNamespace::XLink, "xlink", "show")),
        "xlink:title" => Some((DomNamespace::XLink, "xlink", "title")),
        "xlink:type" => Some((DomNamespace::XLink, "xlink", "type")),
        "xml:lang" => Some((DomNamespace::XML, "xml", "lang")),
        "xml:space" => Some((DomNamespace::XML, "xml", "space")),
        "xmlns" => Some((DomNamespace::XMLNS, "", "xmlns")),
        "xmlns:xlink" => Some((DomNamespace::XMLNS, "xmlns", "xlink")),
        _ => None,
    }
}

/// https://html.spec.whatwg.org/multipage/parsing.html#adjust-svg-attributes
fn adjust_svg_attribute_name(name: &str) -> &str {
    match name {
        "attributename" => "attributeName",
        "attributetype" => "attributeType",
        "basefrequency" => "baseFrequency",
        "baseprofile" => "baseProfile",
        "calcmode" => "calcMode",
        "clippathunits" => "clipPathUnits",
        "diffuseconstant" => "diffuseConstant",
        "edgemode" => "edgeMode",
        "filterunits" => "filterUnits",
        "glyphref" => "glyphRef",
        "gradienttransform" => "gradientTransform",
        "gradientunits" => "gradientUnits",
        "kernelmatrix" => "kernelMatrix",
        "kernelunitlength" => "kernelUnitLength",
        "keypoints" => "keyPoints",
        "keysplines" => "keySplines",
        "keytimes" => "keyTimes",
        "lengthadjust" => "lengthAdjust",
        "limitingconeangle" => "limitingConeAngle",
        "markerheight" => "markerHeight",
        "markerunits" => "markerUnits",
        "markerwidth" => "markerWidth",
        "maskcontentunits" => "maskContentUnits",
        "maskunits" => "maskUnits",
        "numoctaves" => "numOctaves",
        "pathlength" => "pathLength",
        "patterncontentunits" => "patternContentUnits",
        "patterntransform" => "patternTransform",
        "patternunits" => "patternUnits",
        "pointsatx" => "pointsAtX",
        "pointsaty" => "pointsAtY",
        "pointsatz" => "pointsAtZ",
        "preservealpha" => "preserveAlpha",
        "preserveaspectratio" => "preserveAspectRatio",
        "primitiveunits" => "primitiveUnits",
        "refx" => "refX",
        "refy" => "refY",
        "repeatcount" => "repeatCount",
        "repeatdur" => "repeatDur",
        "requiredextensions" => "requiredExtensions",
        "requiredfeatures" => "requiredFeatures",
        "specularconstant" => "specularConstant",
        "specularexponent" => "specularExponent",
        "spreadmethod" => "spreadMethod",
        "startoffset" => "startOffset",
        "stddeviation" => "stdDeviation",
        "stitchtiles" => "stitchTiles",
        "surfacescale" => "surfaceScale",
        "systemlanguage" => "systemLanguage",
        "tablevalues" => "tableValues",
        "targetx" => "targetX",
        "targety" => "targetY",
        "textlength" => "textLength",
        "viewbox" => "viewBox",
        "viewtarget" => "viewTarget",
        "xchannelselector" => "xChannelSelector",
        "ychannelselector" => "yChannelSelector",
        "zoomandpan" => "zoomAndPan",
        _ => name,
    }
}

static QUIRKS_PUBLIC_ID_PREFIXES: &[&str] = &[
    "+//Silmaril//dtd html Pro v0r11 19970101//",
    "-//AS//DTD HTML 3.0 asWedit + extensions//",
    "-//AdvaSoft Ltd//DTD HTML 3.0 asWedit + extensions//",
    "-//IETF//DTD HTML 2.0 Level 1//",
    "-//IETF//DTD HTML 2.0 Level 2//",
    "-//IETF//DTD HTML 2.0 Strict Level 1//",
    "-//IETF//DTD HTML 2.0 Strict Level 2//",
    "-//IETF//DTD HTML 2.0 Strict//",
    "-//IETF//DTD HTML 2.0//",
    "-//IETF//DTD HTML 2.1E//",
    "-//IETF//DTD HTML 3.0//",
    "-//IETF//DTD HTML 3.2 Final//",
    "-//IETF//DTD HTML 3.2//",
    "-//IETF//DTD HTML 3//",
    "-//IETF//DTD HTML Level 0//",
    "-//IETF//DTD HTML Level 1//",
    "-//IETF//DTD HTML Level 2//",
    "-//IETF//DTD HTML Level 3//",
    "-//IETF//DTD HTML Strict Level 0//",
    "-//IETF//DTD HTML Strict Level 1//",
    "-//IETF//DTD HTML Strict Level 2//",
    "-//IETF//DTD HTML Strict Level 3//",
    "-//IETF//DTD HTML Strict//",
    "-//IETF//DTD HTML//",
    "-//Metrius//DTD Metrius Presentational//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 2.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 2.0 Tables//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML Strict//",
    "-//Microsoft//DTD Internet Explorer 3.0 HTML//",
    "-//Microsoft//DTD Internet Explorer 3.0 Tables//",
    "-//Netscape Comm. Corp.//DTD HTML//",
    "-//Netscape Comm. Corp.//DTD Strict HTML//",
    "-//O'Reilly and Associates//DTD HTML 2.0//",
    "-//O'Reilly and Associates//DTD HTML Extended 1.0//",
    "-//O'Reilly and Associates//DTD HTML Extended Relaxed 1.0//",
    "-//SQ//DTD HTML 2.0 HoTMetaL + extensions//",
    "-//SoftQuad Software//DTD HoTMetaL PRO 6.0::19990601::extensions to HTML 4.0//",
    "-//SoftQuad//DTD HoTMetaL PRO 4.0::19971010::extensions to HTML 4.0//",
    "-//Spyglass//DTD HTML 2.0 Extended//",
    "-//Sun Microsystems Corp.//DTD HotJava HTML//",
    "-//Sun Microsystems Corp.//DTD HotJava Strict HTML//",
    "-//W3C//DTD HTML 3 1995-03-24//",
    "-//W3C//DTD HTML 3.2 Draft//",
    "-//W3C//DTD HTML 3.2 Final//",
    "-//W3C//DTD HTML 3.2//",
    "-//W3C//DTD HTML 3.2S Draft//",
    "-//W3C//DTD HTML 4.0 Frameset//",
    "-//W3C//DTD HTML 4.0 Transitional//",
    "-//W3C//DTD HTML Experimental 19960712//",
    "-//W3C//DTD HTML Experimental 970421//",
    "-//W3C//DTD W3 HTML//",
    "-//W3O//DTD W3 HTML 3.0//",
    "-//WebTechs//DTD Mozilla HTML 2.0//",
    "-//WebTechs//DTD Mozilla HTML//",
];
