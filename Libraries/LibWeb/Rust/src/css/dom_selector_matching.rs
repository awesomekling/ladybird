/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Selector matching for the DOM query APIs (`querySelector()`, `querySelectorAll()`, `matches()` and `closest()`),
//! against the DOM itself.
//!
//! The DOM is main-thread state with one writer, so a query reads it where it stands: nothing is mirrored for it, and
//! nothing a style pass owns is read. Every fact a selector tests is asked of the DOM through the callbacks the host
//! passes in.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

use super::css_tokenizer::TokenizerInput;
use super::ffi_support::FfiUtf16View;
use super::selector::{
    AttributeCaseType, AttributeSelector, Combinator, CompiledSelector, Direction, NamespaceType, PseudoClassSelector,
    PseudoClassType, QualifiedName, RustSelector, SimpleSelector, is_ascii_case_insensitive_html_attribute,
    language_range_matches_tag,
};
use super::style::selector::{AttributeOperator, attribute_value_matches};

/// The names of one element a selector compares, as interned string identities.
#[repr(C)]
pub struct FfiDomElementNames {
    pub local_name: usize,
    /// Zero for the null namespace.
    pub namespace_uri: usize,
    /// Zero for an element with no id.
    pub id: usize,
    pub classes: *const usize,
    pub class_count: usize,
    pub is_html_element_in_html_document: bool,
    /// Whether ids and classes compare ASCII case-insensitively, as they do in a quirks-mode document.
    pub ids_and_classes_ignore_case: bool,
    pub is_document_element: bool,
}

/// Which of an element's element siblings a child-indexed pseudo-class counts.
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum FfiSiblingCount {
    Before,
    After,
    /// Those before the element with its local name and namespace.
    BeforeOfSameType,
    /// Those after the element with its local name and namespace.
    AfterOfSameType,
}

/// What the matcher asks of the DOM. Every element pointer is a live element for the duration of the query, and every
/// element a callback returns is one too, or null for none.
#[repr(C)]
pub struct FfiDomSelectorCallbacks {
    pub element_names: unsafe extern "C" fn(element: *const c_void) -> FfiDomElementNames,
    pub parent_element: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    /// The host of the shadow root that is the element's parent, or null when its parent is not a shadow root.
    pub host_of_parent_shadow_root: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    pub previous_element_sibling: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    pub next_element_sibling: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    pub first_element_child: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    pub count_element_siblings: unsafe extern "C" fn(element: *const c_void, which: FfiSiblingCount) -> u32,
    /// The first element after `node` in tree order that is a descendant of `root`. `node` is `root` itself to start.
    /// Both may be any node.
    pub next_element_in_subtree: unsafe extern "C" fn(node: *const c_void, root: *const c_void) -> *const c_void,
    /// Calls `visit` with the value of each attribute of the element whose local name is `name`: the one in no
    /// namespace, or those in every namespace when `any_namespace` is set. Stops once `visit` returns true, and
    /// returns whether it did. Each value is only valid during its `visit`.
    pub visit_attribute_values: unsafe extern "C" fn(
        element: *const c_void,
        name: usize,
        any_namespace: bool,
        context: *mut c_void,
        visit: unsafe extern "C" fn(context: *mut c_void, value: FfiUtf16View) -> bool,
    ) -> bool,
    /// Whether the element's id (or one of its classes, when `is_class` is set) is `name`, compared ASCII
    /// case-insensitively.
    pub id_or_class_equals_ignoring_ascii_case:
        unsafe extern "C" fn(element: *const c_void, is_class: bool, name: usize) -> bool,
    /// Whether the element is in the state a boolean pseudo-class names, by its FFI value.
    pub matches_state: unsafe extern "C" fn(element: *const c_void, pseudo_class: u8) -> bool,
    /// The element's resolved language tag, or an empty view when it has none. Only valid until the next callback.
    pub language: unsafe extern "C" fn(element: *const c_void) -> FfiUtf16View,
    pub is_right_to_left: unsafe extern "C" fn(element: *const c_void) -> bool,
    /// The element's heading level, or zero when it is not a heading.
    pub heading_level: unsafe extern "C" fn(element: *const c_void) -> u32,
    pub has_custom_state: unsafe extern "C" fn(element: *const c_void, state: usize) -> bool,
    /// Whether no child of the element keeps it from being `:empty`.
    pub is_empty: unsafe extern "C" fn(element: *const c_void) -> bool,
}

/// One selector query: the selector list, and the context `:scope` and `:host` are resolved in.
#[repr(C)]
pub struct FfiDomSelectorQuery {
    pub selectors: *const *const RustSelector,
    pub selector_count: usize,
    pub callbacks: *const FfiDomSelectorCallbacks,
    /// The element `:scope` names, or null when the query is rooted at a document, a shadow root or a fragment.
    pub scope: *const c_void,
    /// The host of the shadow tree the query is made in, or null outside one.
    pub shadow_host: *const c_void,
}

type Element = *const c_void;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectorKind {
    Normal,
    /// A `:has()` argument, whose leftmost compound is the implied anchor.
    Relative,
}

#[derive(Clone, Copy)]
struct MatchState {
    shadow_host: Option<Element>,
    selector_kind: SelectorKind,
    /// The `:has()` subject. Right-to-left traversal must not cross or match this element.
    anchor: Option<Element>,
}

struct DomMatcher<'a> {
    dom: &'a FfiDomSelectorCallbacks,
    scope: Option<Element>,
    shadow_host: Option<Element>,
    /// Answers of `:has()` arguments, by selector identity and anchor.
    has_answers: HashMap<(u64, usize), bool>,
    /// The compounds of a selector, by address and index, found not to match through their combinator at an element.
    /// A descendant or subsequent-sibling combinator tries the compounds before it at every ancestor or earlier
    /// sibling, and whether they match there does not depend on the element the walk came from: remembering none of
    /// it, `p div div div span` beside no `p` tries every combination of ancestors for its `div`s.
    relation_failures: HashSet<(usize, usize, usize)>,
}

fn optional_element(element: *const c_void) -> Option<Element> {
    (!element.is_null()).then_some(element)
}

impl<'a> DomMatcher<'a> {
    fn new(dom: &'a FfiDomSelectorCallbacks, scope: Option<Element>, shadow_host: Option<Element>) -> Self {
        Self {
            dom,
            scope,
            shadow_host,
            has_answers: HashMap::new(),
            relation_failures: HashSet::new(),
        }
    }

    fn top_level_state(&self) -> MatchState {
        MatchState {
            shadow_host: self.shadow_host,
            selector_kind: SelectorKind::Normal,
            anchor: None,
        }
    }

    // The callbacks, each called on live elements only.

    fn names(&self, element: Element) -> FfiDomElementNames {
        unsafe { (self.dom.element_names)(element) }
    }

    fn parent_element(&self, element: Element, shadow_host: Option<Element>) -> Option<Element> {
        // The walk up out of a shadow tree ends at its host.
        if Some(element) == shadow_host {
            return None;
        }
        if let Some(parent) = optional_element(unsafe { (self.dom.parent_element)(element) }) {
            return Some(parent);
        }
        // Within a shadow tree, the walk up leaves the tree for its host, which is featureless there.
        shadow_host?;
        optional_element(unsafe { (self.dom.host_of_parent_shadow_root)(element) })
            .filter(|&host| Some(host) == shadow_host)
    }

    fn previous_element_sibling(&self, element: Element) -> Option<Element> {
        optional_element(unsafe { (self.dom.previous_element_sibling)(element) })
    }

    fn next_element_sibling(&self, element: Element) -> Option<Element> {
        optional_element(unsafe { (self.dom.next_element_sibling)(element) })
    }

    fn count_element_siblings(&self, element: Element, which: FfiSiblingCount) -> u32 {
        unsafe { (self.dom.count_element_siblings)(element, which) }
    }

    fn first_element_child(&self, element: Element) -> Option<Element> {
        optional_element(unsafe { (self.dom.first_element_child)(element) })
    }

    fn next_element_in_subtree(&self, node: Element, root: Element) -> Option<Element> {
        optional_element(unsafe { (self.dom.next_element_in_subtree)(node, root) })
    }

    fn matches_selector(&mut self, selector: &CompiledSelector, element: Element, state: MatchState) -> bool {
        // A query names elements, and a pseudo-element is not one.
        if selector.target_pseudo_element.is_some() {
            return false;
        }
        let Some(last) = selector.compound_selectors.len().checked_sub(1) else {
            return false;
        };
        self.matches_compound(selector, last, element, state)
    }

    // https://drafts.csswg.org/selectors-4/#match-a-selector-against-an-element
    fn matches_compound(
        &mut self,
        selector: &CompiledSelector,
        index: usize,
        element: Element,
        state: MatchState,
    ) -> bool {
        let compound = &selector.compound_selectors[index];
        let is_has = |simple: &&SimpleSelector| matches!(simple, SimpleSelector::PseudoClass(pseudo_class) if pseudo_class.pseudo_class == PseudoClassType::Has);
        // OPTIMIZATION: Evaluate :has() last. Its subtree traversal is substantially more expensive than the other
        //               simple selectors and cannot affect their result.
        for simple in compound.simple_selectors.iter().filter(|simple| !is_has(simple)) {
            if !self.matches_simple(simple, element, state) {
                return false;
            }
        }
        for simple in compound.simple_selectors.iter().filter(is_has) {
            if !self.matches_simple(simple, element, state) {
                return false;
            }
        }

        if state.selector_kind == SelectorKind::Relative && index == 0 {
            return Some(element) != state.anchor;
        }

        match compound.combinator {
            Combinator::None => state.selector_kind != SelectorKind::Relative,
            Combinator::ImmediateChild => {
                let Some(parent) = self.parent_element(element, state.shadow_host) else {
                    return false;
                };
                Some(parent) != state.anchor && self.matches_compound(selector, index - 1, parent, state)
            }
            Combinator::NextSibling => {
                let Some(sibling) = self.previous_element_sibling(element) else {
                    return false;
                };
                self.matches_compound(selector, index - 1, sibling, state)
            }
            Combinator::Descendant | Combinator::SubsequentSibling => {
                let remembers_failures = state.anchor.is_none()
                    && state.selector_kind == SelectorKind::Normal
                    && state.shadow_host == self.shadow_host;
                let failure_key = (std::ptr::from_ref(selector) as usize, index, element as usize);
                if remembers_failures && self.relation_failures.contains(&failure_key) {
                    return false;
                }
                let matched = if compound.combinator == Combinator::Descendant {
                    let mut ancestor = self.parent_element(element, state.shadow_host);
                    loop {
                        let Some(candidate) = ancestor else {
                            break false;
                        };
                        if Some(candidate) == state.anchor {
                            break false;
                        }
                        if self.matches_compound(selector, index - 1, candidate, state) {
                            break true;
                        }
                        ancestor = self.parent_element(candidate, state.shadow_host);
                    }
                } else {
                    let mut sibling = self.previous_element_sibling(element);
                    loop {
                        let Some(candidate) = sibling else {
                            break false;
                        };
                        if self.matches_compound(selector, index - 1, candidate, state) {
                            break true;
                        }
                        sibling = self.previous_element_sibling(candidate);
                    }
                };
                if !matched && remembers_failures {
                    self.relation_failures.insert(failure_key);
                }
                matched
            }
            // A pseudo-element is never what a query names, and nothing matches across a column combinator.
            Combinator::PseudoElement | Combinator::Column => false,
        }
    }

    fn matches_simple(&mut self, simple: &SimpleSelector, element: Element, state: MatchState) -> bool {
        // https://drafts.csswg.org/css-scoping-1/#host-element-in-tree
        // When considered within its own shadow trees, the shadow host is featureless. Only the :host, :host(), and
        // :host-context() pseudo-classes are allowed to match it.
        //
        // NB: :has(), :is() and :where() are admitted here because they may contain :host. Their inner selectors are
        //     checked independently and cannot make the host non-featureless.
        if state.shadow_host == Some(element)
            && !matches!(simple, SimpleSelector::PseudoClass(pseudo_class) if matches!(
                pseudo_class.pseudo_class,
                PseudoClassType::Host | PseudoClassType::Has | PseudoClassType::Is | PseudoClassType::Where
            ))
        {
            return false;
        }

        match simple {
            SimpleSelector::Universal(name) => self.matches_namespace(element, name),
            SimpleSelector::TagName(name) => self.matches_tag_name(element, name),
            SimpleSelector::Id(id) => {
                let names = self.names(element);
                if names.id == 0 {
                    return false;
                }
                if names.ids_and_classes_ignore_case {
                    return id.interned_name_identity().is_some_and(|id| unsafe {
                        (self.dom.id_or_class_equals_ignoring_ascii_case)(element, false, id)
                    });
                }
                id.interned_name_identity() == Some(names.id)
            }
            SimpleSelector::Class(class_name) => {
                let names = self.names(element);
                if names.class_count == 0 {
                    return false;
                }
                let Some(identity) = class_name.interned_name_identity() else {
                    return false;
                };
                if names.ids_and_classes_ignore_case {
                    return unsafe { (self.dom.id_or_class_equals_ignoring_ascii_case)(element, true, identity) };
                }
                unsafe { std::slice::from_raw_parts(names.classes, names.class_count) }.contains(&identity)
            }
            SimpleSelector::Attribute(attribute) => self.matches_attribute(element, attribute),
            SimpleSelector::PseudoClass(pseudo_class) => self.matches_pseudo_class(pseudo_class, element, state),
            // A query names elements, and a pseudo-element is not one.
            SimpleSelector::PseudoElement(_) => false,
            // The nesting selector has no parent rule in a query, so it names the scoping root, as `:scope` does.
            SimpleSelector::Nesting => self.matches_scope(element),
            SimpleSelector::Invalid(_) => false,
        }
    }

    // A query resolves no namespace prefix: a default namespace does not exist there, and a named one does not parse.
    fn matches_namespace(&self, element: Element, name: &QualifiedName) -> bool {
        match name.namespace_type {
            NamespaceType::Default | NamespaceType::Any => true,
            NamespaceType::None => self.names(element).namespace_uri == 0,
            NamespaceType::Named => false,
        }
    }

    // https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors
    fn matches_tag_name(&self, element: Element, name: &QualifiedName) -> bool {
        let names = self.names(element);
        // When comparing a CSS element type selector to the names of HTML elements in HTML documents, the CSS element
        // type selector must first be converted to ASCII lowercase. The same selector when compared to other elements
        // must be compared according to its original case. In both cases, to match, the values must be identical to
        // each other (and therefore the comparison is case sensitive).
        let name_matches = if names.is_html_element_in_html_document {
            name.interned_lowercase_name_identity() == Some(names.local_name)
        } else {
            name.interned_name_identity() == Some(names.local_name)
        };
        name_matches && self.matches_namespace(element, name)
    }

    fn matches_attribute(&self, element: Element, attribute: &AttributeSelector) -> bool {
        let qualified_name = &attribute.qualified_name;
        let any_namespace = match qualified_name.namespace_type {
            // https://www.w3.org/TR/selectors-4/#attrnmsp
            // Default namespaces do not apply to attributes, therefore attribute selectors without a namespace
            // component apply only to attributes that have no namespace (equivalent to "|attr").
            NamespaceType::Default | NamespaceType::None => false,
            NamespaceType::Any => true,
            NamespaceType::Named => return false,
        };
        let is_html_element_in_html_document = self.names(element).is_html_element_in_html_document;
        let name = if is_html_element_in_html_document {
            qualified_name.interned_lowercase_name_identity()
        } else {
            qualified_name.interned_name_identity()
        };
        let Some(name) = name else {
            return false;
        };
        let insensitive = match attribute.case_type {
            AttributeCaseType::Insensitive => true,
            AttributeCaseType::Sensitive => false,
            AttributeCaseType::Default => {
                is_html_element_in_html_document
                    && qualified_name.namespace_type != NamespaceType::Any
                    && is_ascii_case_insensitive_html_attribute(&qualified_name.name)
            }
        };

        struct Visit<'a> {
            operator: AttributeOperator,
            literal: &'a [u16],
            insensitive: bool,
        }
        unsafe extern "C" fn visit(context: *mut c_void, value: FfiUtf16View) -> bool {
            let visit = unsafe { &*context.cast::<Visit<'_>>() };
            match unsafe { value.units() } {
                Some(TokenizerInput::Ascii(value)) => {
                    attribute_value_matches(visit.operator, value, visit.literal, visit.insensitive)
                }
                Some(TokenizerInput::Utf16(value)) => {
                    attribute_value_matches(visit.operator, value, visit.literal, visit.insensitive)
                }
                None => attribute_value_matches::<u16>(visit.operator, &[], visit.literal, visit.insensitive),
            }
        }
        let mut context = Visit {
            operator: AttributeOperator::from(attribute.match_type),
            literal: &attribute.value,
            insensitive,
        };
        unsafe {
            (self.dom.visit_attribute_values)(
                element,
                name,
                any_namespace,
                std::ptr::from_mut(&mut context).cast(),
                visit,
            )
        }
    }

    fn matches_scope(&self, element: Element) -> bool {
        // A query rooted at a document, a shadow root or a fragment has no scoping element for `:scope` to name.
        self.scope == Some(element)
    }

    fn matches_pseudo_class(
        &mut self,
        pseudo_class: &PseudoClassSelector,
        element: Element,
        state: MatchState,
    ) -> bool {
        use PseudoClassType::*;

        match pseudo_class.pseudo_class {
            // https://drafts.csswg.org/selectors/#matches
            // Both are forgiving, so an argument list whose every selector is invalid parses to an empty one, and an
            // empty list matches nothing.
            Is | Where => pseudo_class.argument_selector_list.iter().any(|selector| {
                self.matches_selector(
                    selector,
                    element,
                    MatchState {
                        selector_kind: SelectorKind::Normal,
                        anchor: None,
                        ..state
                    },
                )
            }),
            Not => pseudo_class.argument_selector_list.iter().all(|selector| {
                !self.matches_selector(
                    selector,
                    element,
                    MatchState {
                        selector_kind: SelectorKind::Normal,
                        anchor: None,
                        ..state
                    },
                )
            }),
            Has => {
                // https://drafts.csswg.org/selectors-4/#relational
                // The relational pseudo-class, :has(), is a functional pseudo-class taking a <relative-selector-list>
                // as an argument. It represents an element if any of the relative selectors would match at least one
                // element when anchored against this element.
                if state.selector_kind == SelectorKind::Relative {
                    return false;
                }
                pseudo_class
                    .argument_selector_list
                    .iter()
                    .any(|selector| self.matches_has_argument(selector, element, state.shadow_host))
            }
            Host => {
                // https://drafts.csswg.org/css-scoping-1/#host-selector
                // When evaluated in the context of a shadow tree, it matches the shadow tree's shadow host if the
                // shadow host, in its normal context, matches the selector argument. In any other context, it matches
                // nothing.
                if state.shadow_host != Some(element) {
                    return false;
                }
                pseudo_class.argument_selector_list.first().is_none_or(|selector| {
                    self.matches_selector(
                        selector,
                        element,
                        MatchState {
                            shadow_host: None,
                            selector_kind: SelectorKind::Normal,
                            anchor: None,
                        },
                    )
                })
            }
            Scope => self.matches_scope(element),
            Root => self.names(element).is_document_element,
            Empty => unsafe { (self.dom.is_empty)(element) },
            FirstChild => self.previous_element_sibling(element).is_none(),
            LastChild => self.next_element_sibling(element).is_none(),
            OnlyChild => {
                self.previous_element_sibling(element).is_none() && self.next_element_sibling(element).is_none()
            }
            FirstOfType => self.count_element_siblings(element, FfiSiblingCount::BeforeOfSameType) == 0,
            LastOfType => self.count_element_siblings(element, FfiSiblingCount::AfterOfSameType) == 0,
            OnlyOfType => {
                self.count_element_siblings(element, FfiSiblingCount::BeforeOfSameType) == 0
                    && self.count_element_siblings(element, FfiSiblingCount::AfterOfSameType) == 0
            }
            NthChild | NthLastChild | NthOfType | NthLastOfType => self.matches_nth(pseudo_class, element, state),
            Lang => {
                let language = unsafe { (self.dom.language)(element) };
                let Some(language) = (unsafe { language.to_utf16() }) else {
                    return false;
                };
                // An element with no resolved language matches no range at all, not even `*`.
                !language.is_empty()
                    && pseudo_class
                        .languages
                        .iter()
                        .any(|range| language_range_matches_tag(&range.value, &language))
            }
            Dir => match pseudo_class.direction {
                Some(Direction::LeftToRight) => !unsafe { (self.dom.is_right_to_left)(element) },
                Some(Direction::RightToLeft) => unsafe { (self.dom.is_right_to_left)(element) },
                Some(Direction::Other) | None => false,
            },
            State => pseudo_class
                .identifier_identity
                .optional_raw()
                .is_some_and(|state| unsafe { (self.dom.has_custom_state)(element, state) }),
            Heading => {
                // A written heading is one of six, but its computed level counts the heading offset its ancestors
                // declare and is clamped at nine. Bare `:heading` is any level an element can have.
                let level = unsafe { (self.dom.heading_level)(element) };
                level != 0 && (pseudo_class.levels.is_empty() || pseudo_class.levels.contains(&i64::from(level)))
            }
            // Every other pseudo-class is a boolean state the element is in or not.
            other => unsafe { (self.dom.matches_state)(element, other as u8) },
        }
    }

    // https://drafts.csswg.org/selectors-4/#child-index
    fn matches_nth(&mut self, pseudo_class: &PseudoClassSelector, element: Element, state: MatchState) -> bool {
        let (from_end, of_same_type) = match pseudo_class.pseudo_class {
            PseudoClassType::NthChild => (false, false),
            PseudoClassType::NthLastChild => (true, false),
            PseudoClassType::NthOfType => (false, true),
            PseudoClassType::NthLastOfType => (true, true),
            _ => {
                debug_assert!(false, "not a child-indexed pseudo-class");
                return false;
            }
        };
        // Only :nth-child() and :nth-last-child() take `of S`.
        let siblings_before = if of_same_type || pseudo_class.argument_selector_list.is_empty() {
            let which = match (from_end, of_same_type) {
                (false, false) => FfiSiblingCount::Before,
                (true, false) => FfiSiblingCount::After,
                (false, true) => FfiSiblingCount::BeforeOfSameType,
                (true, true) => FfiSiblingCount::AfterOfSameType,
            };
            self.count_element_siblings(element, which)
        } else {
            // `of S` counts the siblings S matches, so each one is matched here.
            let argument_state = MatchState {
                selector_kind: SelectorKind::Normal,
                anchor: None,
                ..state
            };
            let matches_argument = |this: &mut Self, candidate: Element| {
                pseudo_class
                    .argument_selector_list
                    .iter()
                    .any(|selector| this.matches_selector(selector, candidate, argument_state))
            };
            if !matches_argument(self, element) {
                return false;
            }
            let step = |this: &Self, candidate: Element| match from_end {
                true => this.next_element_sibling(candidate),
                false => this.previous_element_sibling(candidate),
            };
            let mut count = 0u32;
            let mut sibling = step(self, element);
            while let Some(candidate) = sibling {
                if matches_argument(self, candidate) {
                    count += 1;
                }
                sibling = step(self, candidate);
            }
            count
        };
        let index = i32::try_from(siblings_before).unwrap_or(i32::MAX).saturating_add(1);
        pseudo_class.an_plus_b_pattern.matches(index)
    }

    fn matches_has_argument(
        &mut self,
        selector: &CompiledSelector,
        anchor: Element,
        shadow_host: Option<Element>,
    ) -> bool {
        let key = (selector.id(), anchor as usize);
        if let Some(&answer) = self.has_answers.get(&key) {
            return answer;
        }
        let answer = self.matches_relative_selector(selector, 0, anchor, anchor, shadow_host);
        self.has_answers.insert(key, answer);
        answer
    }

    // https://drafts.csswg.org/selectors-4/#relative
    // Relative selectors begin with a combinator, with a selector representing the anchor element implied at the start
    // of the selector. (If no combinator is present, the descendant combinator is implied.)
    //
    // NB: This walks left-to-right from that implied anchor to enumerate candidates. Once a candidate is found,
    //     matches_compound() verifies the corresponding compound right-to-left, preserving the normal matching
    //     semantics for the rest of the selector.
    fn matches_relative_selector(
        &mut self,
        selector: &CompiledSelector,
        index: usize,
        element: Element,
        anchor: Element,
        shadow_host: Option<Element>,
    ) -> bool {
        let state = MatchState {
            shadow_host,
            selector_kind: SelectorKind::Relative,
            anchor: Some(anchor),
        };
        if index >= selector.compound_selectors.len() {
            return self.matches_selector(selector, element, state);
        }
        let matches_here = |this: &mut Self, candidate: Element| {
            this.matches_compound(selector, index, candidate, state)
                && this.matches_relative_selector(selector, index + 1, candidate, anchor, shadow_host)
        };
        match selector.compound_selectors[index].combinator {
            Combinator::Descendant => {
                let mut descendant = self.next_element_in_subtree(element, element);
                while let Some(candidate) = descendant {
                    if self.matches_selector(selector, candidate, state) {
                        return true;
                    }
                    descendant = self.next_element_in_subtree(candidate, element);
                }
                false
            }
            Combinator::ImmediateChild => {
                let mut child = self.first_element_child(element);
                while let Some(candidate) = child {
                    if matches_here(self, candidate) {
                        return true;
                    }
                    child = self.next_element_sibling(candidate);
                }
                false
            }
            Combinator::NextSibling => self
                .next_element_sibling(element)
                .is_some_and(|sibling| matches_here(self, sibling)),
            Combinator::SubsequentSibling => {
                let mut sibling = self.next_element_sibling(element);
                while let Some(candidate) = sibling {
                    if matches_here(self, candidate) {
                        return true;
                    }
                    sibling = self.next_element_sibling(candidate);
                }
                false
            }
            Combinator::None | Combinator::PseudoElement | Combinator::Column => false,
        }
    }

    fn matches_any(&mut self, selectors: &[&CompiledSelector], element: Element) -> bool {
        let state = self.top_level_state();
        selectors
            .iter()
            .any(|selector| self.matches_selector(selector, element, state))
    }
}

unsafe fn query_parts(query: &FfiDomSelectorQuery) -> (Vec<&CompiledSelector>, DomMatcher<'_>) {
    let selectors = if query.selector_count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(query.selectors, query.selector_count) }
            .iter()
            .map(|&selector| unsafe { (*selector).compiled() })
            .collect()
    };
    let matcher = DomMatcher::new(
        unsafe { &*query.callbacks },
        optional_element(query.scope),
        optional_element(query.shadow_host),
    );
    (selectors, matcher)
}

/// Whether an element matches any selector of a query.
///
/// # Safety
/// `query` must describe live selectors and callbacks, and `element` must be a live element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_query_matches(query: &FfiDomSelectorQuery, element: *const c_void) -> bool {
    let (selectors, mut matcher) = unsafe { query_parts(query) };
    matcher.matches_any(&selectors, element)
}

/// The nearest inclusive ancestor of an element that matches any selector of a query, or null.
///
/// # Safety
/// As for `rust_dom_selector_query_matches`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_query_closest(
    query: &FfiDomSelectorQuery,
    element: *const c_void,
) -> *const c_void {
    let (selectors, mut matcher) = unsafe { query_parts(query) };
    let mut candidate = Some(element);
    while let Some(current) = candidate {
        if matcher.matches_any(&selectors, current) {
            return current;
        }
        candidate = matcher.parent_element(current, None);
    }
    std::ptr::null()
}

/// Calls `found` with each element under `root` that matches any selector of a query, in tree order, until it returns
/// true.
///
/// # Safety
/// As for `rust_dom_selector_query_matches`, and `root` must be a live node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_query_subtree(
    query: &FfiDomSelectorQuery,
    root: *const c_void,
    context: *mut c_void,
    found: unsafe extern "C" fn(context: *mut c_void, element: *const c_void) -> bool,
) {
    let (selectors, mut matcher) = unsafe { query_parts(query) };
    let mut candidate = matcher.next_element_in_subtree(root, root);
    while let Some(element) = candidate {
        if matcher.matches_any(&selectors, element) && unsafe { found(context, element) } {
            return;
        }
        candidate = matcher.next_element_in_subtree(element, root);
    }
}
