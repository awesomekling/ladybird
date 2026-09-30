/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Selector matching for the DOM query APIs (`querySelector()`, `querySelectorAll()`, `matches()` and `closest()`),
//! against the DOM itself.
//!
//! A query's selectors compile into a selector program whose atoms are the query's own names, and the one selector
//! evaluator runs it with the live DOM as its subject. The DOM is main-thread state with one writer, so a query reads
//! it where it stands: nothing is mirrored for it, and nothing a style pass owns is read. Every fact a selector tests
//! is asked of the DOM through the callbacks the host passes in.

use std::borrow::Cow;
use std::convert::Infallible;
use std::ffi::c_void;

use smallvec::SmallVec;

use super::css_tokenizer::TokenizerInput;
use super::ffi_support::FfiUtf16View;
use super::selector::RustSelector;
use super::style::compiler::SelectorCompiler;
use super::style::fast_hash::FastMap as HashMap;
use super::style::index::DispatchKey;
use super::style::index::StyleAtomID;
use super::style::relative_selector::RelativeQueryID;
use super::style::selector::FeatureTest;
use super::style::selector::NthPosition;
use super::style::selector::QueryAtoms;
use super::style::selector::SelectorNodeID;
use super::style::selector::SelectorOp;
use super::style::selector::SelectorProgram;
use super::style::selector_evaluation::ElementFeatures;
use super::style::selector_evaluation::PrecedingSiblingPrefix;
use super::style::selector_evaluation::RememberedPrefix;
use super::style::selector_evaluation::SelectorBindings;
use super::style::selector_evaluation::SelectorEvaluator;
use super::style::selector_evaluation::SelectorSubject;
use super::style::selector_evaluation::SelectorTree;
use super::style::transaction::StateFact;

/// What a selector compares of one element: its names, as interned string identities, and how many of its attributes
/// have a local name the matcher asked for.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiDomElement {
    pub local_name: usize,
    /// Zero for the null namespace.
    pub namespace_uri: usize,
    /// Zero for an element with no id.
    pub id: usize,
    pub classes: *const usize,
    pub class_count: usize,
    pub attribute_count: usize,
}

/// One attribute of an element, borrowed from the DOM until it next changes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiDomAttribute {
    pub local_name: usize,
    /// Zero for the null namespace.
    pub namespace_uri: usize,
    pub value: FfiUtf16View,
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

/// What the matcher asks of the DOM. Every node pointer is a live node for the duration of the query, and every node a
/// callback returns is one too, or null for none.
#[repr(C)]
pub struct FfiDomSelectorCallbacks {
    /// The element, with the first `capacity` of its attributes whose local name is one of the `name_count` in `names`
    /// written to `attributes`.
    pub element: unsafe extern "C" fn(
        element: *const c_void,
        names: *const usize,
        name_count: usize,
        attributes: *mut FfiDomAttribute,
        capacity: usize,
    ) -> FfiDomElement,
    /// The node's parent as selectors see it: its parent element, or the shadow root it is a child of. Null for a
    /// shadow root, and for a child of a document or a fragment.
    pub parent: unsafe extern "C" fn(node: *const c_void) -> *const c_void,
    pub previous_element_sibling: unsafe extern "C" fn(node: *const c_void) -> *const c_void,
    pub next_element_sibling: unsafe extern "C" fn(node: *const c_void) -> *const c_void,
    pub first_element_child: unsafe extern "C" fn(node: *const c_void) -> *const c_void,
    /// The first element child of the node's parent, whatever node that is, or the node itself when it has none.
    pub first_element_sibling: unsafe extern "C" fn(node: *const c_void) -> *const c_void,
    pub count_element_siblings: unsafe extern "C" fn(element: *const c_void, which: FfiSiblingCount) -> u32,
    /// The first element after `node` in tree order that is a descendant of `root`, skipping the subtrees whose
    /// attribute name filter lacks one of the bits of `attribute_names`. `node` is `root` itself to start. Both may be
    /// any node.
    pub next_element_in_subtree:
        unsafe extern "C" fn(node: *const c_void, root: *const c_void, attribute_names: u64) -> *const c_void,
    /// The bit of the DOM's attribute name filter for a local name.
    pub attribute_name_filter_bit: unsafe extern "C" fn(local_name: usize) -> u64,
    /// The shadow root the element hosts, or null.
    pub shadow_root: unsafe extern "C" fn(element: *const c_void) -> *const c_void,
    /// The host of a shadow root.
    pub host: unsafe extern "C" fn(shadow_root: *const c_void) -> *const c_void,
    /// Whether the element's id (or one of its classes, when `is_class` is set) is `name`, compared ASCII
    /// case-insensitively.
    pub id_or_class_equals_ignoring_ascii_case:
        unsafe extern "C" fn(element: *const c_void, is_class: bool, name: usize) -> bool,
    /// Whether the element is in a state, by the state's `StateFact` value.
    pub matches_state: unsafe extern "C" fn(element: *const c_void, state: u8) -> bool,
    /// The element's resolved language tag, or an empty view when it has none. Only valid until the next callback.
    pub language: unsafe extern "C" fn(element: *const c_void) -> FfiUtf16View,
    /// The interned identity of the element's directionality: `ltr` or `rtl`.
    pub directionality: unsafe extern "C" fn(element: *const c_void) -> usize,
    /// The element's heading level, or zero when it is not a heading.
    pub heading_level: unsafe extern "C" fn(element: *const c_void) -> u32,
    pub has_custom_state: unsafe extern "C" fn(element: *const c_void, state: usize) -> bool,
    /// Whether no child of the element keeps it from being `:empty`.
    pub is_empty: unsafe extern "C" fn(element: *const c_void) -> bool,
}

/// One selector query: the compiled selectors, and the context `:scope` and `:host` are resolved in.
#[repr(C)]
pub struct FfiDomSelectorQuery {
    pub program: *const DomSelectorProgram,
    pub callbacks: *const FfiDomSelectorCallbacks,
    /// The element `:scope` names: the element the query is scoped to, or the document element for a query scoped to
    /// a document, a shadow root or a fragment. Null for none.
    pub scope: *const c_void,
    /// The shadow root of the tree the query is made in, or null outside one.
    pub shadow_root: *const c_void,
    /// The document element of the document every element the query reads is in, or null for none.
    pub document_element: *const c_void,
    /// Whether ids and classes compare ASCII case-insensitively, as they do in a quirks-mode document.
    pub ids_and_classes_ignore_case: bool,
}

type DomNode = *const c_void;

fn optional_node(node: *const c_void) -> Option<DomNode> {
    (!node.is_null()).then_some(node)
}

/// The names a query's program holds as atoms, which are the raw identities of the interned strings its selectors
/// hold. Atom `n` is entry `n - 1`, so no name is `StyleAtomID::NONE`.
#[derive(Default)]
struct QueryNames(Vec<(usize, Option<StyleAtomID>)>);

impl QueryNames {
    fn intern(&mut self, raw: usize, namespace: Option<StyleAtomID>) -> StyleAtomID {
        let index = match self.0.iter().position(|&name| name == (raw, namespace)) {
            Some(index) => index,
            None => {
                self.0.push((raw, namespace));
                self.0.len() - 1
            }
        };
        StyleAtomID(u32::try_from(index + 1).unwrap_or(u32::MAX))
    }

    /// The raw identity of an atom's name, or zero for `NONE`.
    #[inline]
    fn raw(&self, atom: StyleAtomID) -> usize {
        atom.0
            .checked_sub(1)
            .and_then(|index| self.0.get(index as usize))
            .map_or(0, |&(raw, _)| raw)
    }
}

/// A selector query compiled for one kind of document, as the host caches it with the query.
pub struct DomSelectorProgram {
    program: SelectorProgram<QueryAtoms>,
    names: QueryNames,
    /// Every attribute name the program's selectors compare, in either case: an element's row is read with them.
    attribute_names: Box<[usize]>,
    /// The roots of the entries that can name an element. A query names elements, and a pseudo-element is not one.
    subjects: Box<[SelectorNodeID]>,
    /// The local names of the attributes every subject requires its element to carry.
    required_attribute_names: Box<[usize]>,
}

impl DomSelectorProgram {
    /// The attribute names every match carries, as bits of the DOM's attribute name filter.
    fn required_attribute_name_bits(&self, dom: &FfiDomSelectorCallbacks) -> u64 {
        self.required_attribute_names
            .iter()
            .fold(0, |bits, &name| bits | unsafe { (dom.attribute_name_filter_bit)(name) })
    }
}

/// Compile a selector list for the DOM query APIs, in a document whose HTML elements are in `html_namespace`, which
/// is zero for a document that is not an HTML document.
///
/// # Safety
/// `selectors` must point to `count` live selectors, which must outlive the program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_program_create(
    selectors: *const *const RustSelector,
    count: usize,
    html_namespace: usize,
) -> *mut DomSelectorProgram {
    let selectors = match count {
        0 => &[][..],
        _ => unsafe { std::slice::from_raw_parts(selectors, count) },
    };
    let mut names = QueryNames::default();
    let program = {
        let mut intern = |raw, namespace| names.intern(raw, namespace);
        let html_element_namespace = match html_namespace {
            0 => StyleAtomID::NONE,
            namespace => intern(namespace, None),
        };
        let mut compiler = SelectorCompiler::for_query(&mut intern, html_element_namespace);
        for &selector in selectors {
            compiler.compile(unsafe { (*selector).compiled() });
        }
        compiler.finish()
    };
    let mut attribute_names = Vec::new();
    // An attribute name that folds is carried in one case or the other depending on the element, so the folded form
    // it dispatches on is not the name every match carries.
    let mut folding_names = SmallVec::<[StyleAtomID; 4]>::new();
    for index in 0..program.node_count() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        if let SelectorOp::Feature(FeatureTest::Attribute(test)) = program.node(SelectorNodeID(index)) {
            for name in [names.raw(test.name), names.raw(test.folded)] {
                if !attribute_names.contains(&name) {
                    attribute_names.push(name);
                }
            }
            if test.name != test.folded {
                folding_names.push(test.folded);
            }
        }
    }
    let mut subjects = Vec::new();
    let mut required_attribute_names: Option<Vec<usize>> = None;
    for (index, entry) in program.entries().iter().enumerate() {
        if entry.pseudo_element.is_some() || program.entry_never_matches(entry) {
            continue;
        }
        subjects.push(entry.root);
        // The keys an entry dispatches on are alternatives, so a lone one is required as well.
        let dispatch = program.subject_dispatch_keys(index);
        let required_by_entry = program
            .subject_required_keys(index)
            .iter()
            .chain(dispatch.iter().filter(|_| dispatch.len() == 1))
            .filter_map(|&key| match key {
                DispatchKey::AttributeName(name) if !folding_names.contains(&name) => Some(names.raw(name)),
                _ => None,
            });
        match &mut required_attribute_names {
            None => required_attribute_names = Some(required_by_entry.collect()),
            Some(required) => {
                let required_by_entry: SmallVec<[usize; 4]> = required_by_entry.collect();
                required.retain(|name| required_by_entry.contains(name));
            }
        }
    }
    Box::into_raw(Box::new(DomSelectorProgram {
        program,
        names,
        attribute_names: attribute_names.into_boxed_slice(),
        subjects: subjects.into_boxed_slice(),
        required_attribute_names: required_attribute_names.unwrap_or_default().into_boxed_slice(),
    }))
}

/// # Safety
/// `program` must be null or a program `rust_dom_selector_program_create` returned, which is not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_program_destroy(program: *mut DomSelectorProgram) {
    if !program.is_null() {
        drop(unsafe { Box::from_raw(program) });
    }
}

/// The DOM, as the callbacks show it.
#[derive(Clone, Copy)]
struct DomTree<'q> {
    dom: &'q FfiDomSelectorCallbacks,
}

impl SelectorTree for DomTree<'_> {
    type Node = DomNode;

    #[inline]
    fn parent(self, node: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.parent)(node) })
    }

    #[inline]
    fn previous_sibling(self, node: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.previous_element_sibling)(node) })
    }

    #[inline]
    fn next_sibling(self, node: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.next_element_sibling)(node) })
    }

    #[inline]
    fn first_child(self, parent: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.first_element_child)(parent) })
    }

    #[inline]
    fn first_sibling(self, node: DomNode) -> DomNode {
        unsafe { (self.dom.first_element_sibling)(node) }
    }

    #[inline]
    fn shadow_root_of(self, host: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.shadow_root)(host) })
    }

    #[inline]
    fn host_of(self, shadow_root: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.host)(shadow_root) })
    }

    #[inline]
    fn next_in_subtree(self, node: DomNode, root: DomNode) -> Option<DomNode> {
        optional_node(unsafe { (self.dom.next_element_in_subtree)(node, root, 0) })
    }
}

/// One element's row: the facts one host call reads of it.
#[derive(Clone, Copy)]
struct DomRow {
    node: DomNode,
    element: FfiDomElement,
}

/// What a child-indexed pseudo-class counts among an element's siblings, and from which end.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SiblingCounter {
    from_end: bool,
    /// The local name and namespace of the elements counted, or none to count every element.
    of_type: Option<(usize, usize)>,
}

/// The live DOM as a subject of one query.
struct DomSubject<'q> {
    dom: &'q FfiDomSelectorCallbacks,
    query: &'q DomSelectorProgram,
    document_element: Option<DomNode>,
    ids_and_classes_ignore_case: bool,
    /// The element whose row was read last, whose attributes the program names are in `attributes`.
    current: Option<DomRow>,
    attributes: SmallVec<[FfiDomAttribute; 4]>,
    relation_answers: HashMap<(SelectorNodeID, DomNode), bool>,
    preceding_sibling_prefixes: HashMap<(SelectorNodeID, DomNode), PrecedingSiblingPrefix<DomNode>>,
    relative_answers: HashMap<(RelativeQueryID, DomNode), bool>,
    /// The number of siblings a child-indexed pseudo-class counts through each element, by element and counter. A
    /// count walks only to the nearest sibling it knows, so matching every child of a parent walks its siblings once
    /// rather than once per child.
    sibling_counts: HashMap<(DomNode, SiblingCounter), u32>,
    /// The siblings a count walks past before it reaches one it knows, kept from count to count.
    sibling_walk: Vec<DomNode>,
    /// Whether a child index has been asked for yet. Until then, nothing is remembered.
    sibling_index_asked: bool,
}

impl<'q> DomSubject<'q> {
    #[inline]
    fn names(&self) -> &'q QueryNames {
        &self.query.names
    }

    /// The element, with the attributes the program names read into `attributes`.
    #[inline(never)]
    fn read_element_with_attributes(&mut self, node: DomNode) -> FfiDomElement {
        let dom = self.dom;
        let names = &self.query.attribute_names;
        let read = |attributes: &mut SmallVec<[FfiDomAttribute; 4]>| unsafe {
            (dom.element)(
                node,
                names.as_ptr(),
                names.len(),
                attributes.as_mut_ptr(),
                attributes.capacity(),
            )
        };
        self.attributes.clear();
        let mut element = read(&mut self.attributes);
        if element.attribute_count > self.attributes.capacity() {
            self.attributes.reserve_exact(element.attribute_count);
            element = read(&mut self.attributes);
        }
        // SAFETY: The host writes as many attributes as it has, up to the capacity it is given.
        unsafe {
            self.attributes
                .set_len(element.attribute_count.min(self.attributes.capacity()));
        };
        element
    }

    /// The number of siblings the counter counts from the start (or the end) through `sibling`.
    fn count_through(&mut self, sibling: DomNode, counter: SiblingCounter) -> u32 {
        let tree = self.tree();
        let mut walk = std::mem::take(&mut self.sibling_walk);
        let mut count = 0;
        let mut cursor = Some(sibling);
        while let Some(current) = cursor {
            if let Some(&known) = self.sibling_counts.get(&(current, counter)) {
                count = known;
                break;
            }
            walk.push(current);
            cursor = match counter.from_end {
                true => tree.next_sibling(current),
                false => tree.previous_sibling(current),
            };
        }
        for &current in walk.iter().rev() {
            let counted = counter.of_type.is_none_or(|of_type| {
                let Ok(row) = self.row(current);
                (row.element.local_name, row.element.namespace_uri) == of_type
            });
            count += u32::from(counted);
            self.sibling_counts.insert((current, counter), count);
        }
        walk.clear();
        self.sibling_walk = walk;
        count
    }
}

/// An element's row, with the subject whose names its atoms are and whose buffer holds its attributes.
struct DomFeatures<'s> {
    subject: &'s DomSubject<'s>,
    row: DomRow,
}

impl ElementFeatures for DomFeatures<'_> {
    type Attribute = FfiDomAttribute;

    #[inline]
    fn local_name_is(&self, name: StyleAtomID) -> bool {
        self.row.element.local_name == self.subject.names().raw(name)
    }

    #[inline]
    fn namespace_is(&self, namespace: StyleAtomID) -> bool {
        self.row.element.namespace_uri == self.subject.names().raw(namespace)
    }

    #[inline]
    fn has_id(&self, id: StyleAtomID) -> bool {
        let id = self.subject.names().raw(id);
        if self.row.element.id == 0 || id == 0 {
            return false;
        }
        match self.subject.ids_and_classes_ignore_case {
            true => unsafe { (self.subject.dom.id_or_class_equals_ignoring_ascii_case)(self.row.node, false, id) },
            false => self.row.element.id == id,
        }
    }

    #[inline]
    fn has_class(&self, class: StyleAtomID) -> bool {
        let class = self.subject.names().raw(class);
        if self.row.element.class_count == 0 || class == 0 {
            return false;
        }
        match self.subject.ids_and_classes_ignore_case {
            true => unsafe { (self.subject.dom.id_or_class_equals_ignoring_ascii_case)(self.row.node, true, class) },
            false => unsafe { std::slice::from_raw_parts(self.row.element.classes, self.row.element.class_count) }
                .contains(&class),
        }
    }

    #[inline]
    fn attributes_named(&self, name: StyleAtomID, any_namespace: bool) -> impl Iterator<Item = FfiDomAttribute> + '_ {
        let name = self.subject.names().raw(name);
        // The buffer holds the attributes of the element read last, which is this one: a row is asked of as soon as
        // it is read.
        let attributes = match self.subject.current {
            Some(current) if current.node == self.row.node => &self.subject.attributes[..],
            _ => {
                debug_assert!(
                    false,
                    "the attributes of an element are asked of after another element was read"
                );
                &[]
            }
        };
        attributes
            .iter()
            .copied()
            .filter(move |attribute| attribute.local_name == name && (any_namespace || attribute.namespace_uri == 0))
    }
}

impl<'q> SelectorSubject for DomSubject<'q> {
    type Atoms = QueryAtoms;
    type Node = DomNode;
    type Tree = DomTree<'q>;
    type Row = DomRow;
    type Attribute = FfiDomAttribute;
    type Features<'s>
        = DomFeatures<'s>
    where
        Self: 's;
    type Incomplete = Infallible;
    type Counters = ();
    type PrefixSlot = (SelectorNodeID, DomNode);

    #[inline]
    fn tree(&self) -> DomTree<'q> {
        DomTree { dom: self.dom }
    }

    #[inline]
    fn row(&mut self, node: DomNode) -> Result<DomRow, Infallible> {
        if let Some(current) = self.current
            && current.node == node
        {
            return Ok(current);
        }
        let element = match self.query.attribute_names.is_empty() {
            true => unsafe { (self.dom.element)(node, std::ptr::null(), 0, std::ptr::null_mut(), 0) },
            false => self.read_element_with_attributes(node),
        };
        let row = DomRow { node, element };
        self.current = Some(row);
        Ok(row)
    }

    #[inline]
    fn features(&self, row: DomRow) -> DomFeatures<'_> {
        DomFeatures { subject: self, row }
    }

    #[inline]
    fn same_type(&self, row: DomRow, other: DomRow) -> bool {
        row.element.local_name == other.element.local_name && row.element.namespace_uri == other.element.namespace_uri
    }

    #[inline]
    fn attribute_value_atom(&self, _attribute: FfiDomAttribute) -> StyleAtomID {
        StyleAtomID::NONE
    }

    #[inline]
    fn attribute_value_text(&self, attribute: FfiDomAttribute) -> Option<TokenizerInput<'_>> {
        // An empty value crosses as an empty view.
        Some(unsafe { attribute.value.units() }.unwrap_or(TokenizerInput::Utf16(&[])))
    }

    #[inline]
    fn has_state(&self, row: DomRow, fact: StateFact) -> bool {
        unsafe { (self.dom.matches_state)(row.node, fact as u8) }
    }

    fn language_tag(&self, row: DomRow) -> Cow<'_, [u16]> {
        match unsafe { (self.dom.language)(row.node).units() } {
            Some(TokenizerInput::Utf16(tag)) => Cow::Borrowed(tag),
            Some(TokenizerInput::Ascii(tag)) => Cow::Owned(tag.iter().copied().map(u16::from).collect()),
            None => Cow::Borrowed(&[]),
        }
    }

    #[inline]
    fn directionality_is(&self, row: DomRow, direction: StyleAtomID) -> bool {
        let actual = unsafe { (self.dom.directionality)(row.node) };
        actual == self.names().raw(direction)
    }

    #[inline]
    fn has_custom_state(&self, row: DomRow, state: StyleAtomID) -> bool {
        let state = self.names().raw(state);
        state != 0 && unsafe { (self.dom.has_custom_state)(row.node, state) }
    }

    #[inline]
    fn heading_level(&self, row: DomRow) -> u8 {
        u8::try_from(unsafe { (self.dom.heading_level)(row.node) }).unwrap_or(0)
    }

    #[inline]
    fn is_empty(&mut self, node: DomNode) -> Result<bool, Infallible> {
        Ok(unsafe { (self.dom.is_empty)(node) })
    }

    #[inline]
    fn is_root(&self, node: DomNode) -> bool {
        Some(node) == self.document_element
    }

    /// The scoping root and the shadow root stay bound for the whole query, so only a relative anchor or a `:host()`
    /// argument makes an answer local to one evaluation.
    #[inline]
    fn remembers_relations(&self, bindings: &SelectorBindings<DomNode>) -> bool {
        bindings.relative_anchor.is_none() && !bindings.matching_host_argument
    }

    #[inline]
    fn relation_answer(
        &self,
        _program: &SelectorProgram<QueryAtoms>,
        relation: SelectorNodeID,
        node: DomNode,
    ) -> Option<bool> {
        self.relation_answers.get(&(relation, node)).copied()
    }

    #[inline]
    fn record_relation_answer(
        &mut self,
        _program: &SelectorProgram<QueryAtoms>,
        relation: SelectorNodeID,
        node: DomNode,
        answer: bool,
    ) {
        self.relation_answers.insert((relation, node), answer);
    }

    #[inline]
    fn preceding_sibling_prefix(
        &mut self,
        _program: &SelectorProgram<QueryAtoms>,
        relation: SelectorNodeID,
        parent: DomNode,
    ) -> Option<RememberedPrefix<(SelectorNodeID, DomNode), DomNode>> {
        let slot = (relation, parent);
        Some(RememberedPrefix {
            slot,
            prefix: self.preceding_sibling_prefixes.get(&slot).copied(),
        })
    }

    #[inline]
    fn record_preceding_sibling_prefix(
        &mut self,
        _program: &SelectorProgram<QueryAtoms>,
        _relation: SelectorNodeID,
        slot: (SelectorNodeID, DomNode),
        prefix: PrecedingSiblingPrefix<DomNode>,
    ) {
        self.preceding_sibling_prefixes.insert(slot, prefix);
    }

    fn sibling_index(&mut self, position: NthPosition, node: DomNode) -> Result<Option<i64>, Infallible> {
        let tree = self.tree();
        let nearest = match position.from_end {
            true => tree.next_sibling(node),
            false => tree.previous_sibling(node),
        };
        let Some(nearest) = nearest else {
            return Ok(Some(1));
        };
        let siblings_before = match self.sibling_index_asked {
            // One element asked alone, as matches() and closest() mostly do, is counted by the host in one call.
            false => {
                self.sibling_index_asked = true;
                let which = match (position.from_end, position.of_type) {
                    (false, false) => FfiSiblingCount::Before,
                    (true, false) => FfiSiblingCount::After,
                    (false, true) => FfiSiblingCount::BeforeOfSameType,
                    (true, true) => FfiSiblingCount::AfterOfSameType,
                };
                unsafe { (self.dom.count_element_siblings)(node, which) }
            }
            true => {
                let of_type = match position.of_type {
                    true => {
                        let Ok(row) = self.row(node);
                        Some((row.element.local_name, row.element.namespace_uri))
                    }
                    false => None,
                };
                let counter = SiblingCounter {
                    from_end: position.from_end,
                    of_type,
                };
                self.count_through(nearest, counter)
            }
        };
        Ok(Some(i64::from(siblings_before) + 1))
    }

    #[inline]
    fn relative_answer(
        &self,
        _program: &SelectorProgram<QueryAtoms>,
        query: RelativeQueryID,
        anchor: DomNode,
    ) -> Option<bool> {
        self.relative_answers.get(&(query, anchor)).copied()
    }

    #[inline]
    fn record_relative_answer(
        &mut self,
        _program: &SelectorProgram<QueryAtoms>,
        query: RelativeQueryID,
        anchor: DomNode,
        answer: bool,
        _witness: Option<DomNode>,
    ) {
        self.relative_answers.insert((query, anchor), answer);
    }
}

/// One query as it runs: its program, evaluated against the DOM.
struct DomQuery<'q> {
    query: &'q DomSelectorProgram,
    evaluator: SelectorEvaluator<DomSubject<'q>>,
}

impl<'q> DomQuery<'q> {
    /// # Safety
    /// `query` must describe a live program and callbacks.
    unsafe fn new(query: &'q FfiDomSelectorQuery) -> Self {
        let program = unsafe { &*query.program };
        let shadow_root = optional_node(query.shadow_root);
        Self {
            query: program,
            evaluator: SelectorEvaluator {
                subject: DomSubject {
                    dom: unsafe { &*query.callbacks },
                    query: program,
                    document_element: optional_node(query.document_element),
                    ids_and_classes_ignore_case: query.ids_and_classes_ignore_case,
                    current: None,
                    attributes: SmallVec::new(),
                    relation_answers: HashMap::default(),
                    preceding_sibling_prefixes: HashMap::default(),
                    relative_answers: HashMap::default(),
                    sibling_counts: HashMap::default(),
                    sibling_walk: Vec::new(),
                    sibling_index_asked: false,
                },
                bindings: SelectorBindings {
                    scope_shadow_root: shadow_root,
                    scope_root_instance: optional_node(query.scope),
                    ..SelectorBindings::default()
                },
            },
        }
    }

    fn matches(&mut self, element: DomNode) -> bool {
        let query = self.query;
        query.subjects.iter().any(|&subject| {
            let Ok(matches) = self.evaluator.matches_node(&query.program, subject, element, &mut ());
            matches
        })
    }
}

/// Whether an element matches any selector of a query.
///
/// # Safety
/// `query` must describe a live program and callbacks, and `element` must be a live element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_dom_selector_query_matches(query: &FfiDomSelectorQuery, element: *const c_void) -> bool {
    unsafe { DomQuery::new(query) }.matches(element)
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
    let mut query_run = unsafe { DomQuery::new(query) };
    let tree = query_run.evaluator.subject.tree();
    let shadow_root = query_run.evaluator.bindings.scope_shadow_root;
    let mut candidate = Some(element);
    // The walk up ends at the root of the element's tree, which is not an element when it is a shadow root.
    while let Some(current) = candidate
        && Some(current) != shadow_root
    {
        if query_run.matches(current) {
            return current;
        }
        candidate = tree.parent(current);
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
    let mut query_run = unsafe { DomQuery::new(query) };
    // A query whose every selector matches nothing needs no walk.
    if query_run.query.subjects.is_empty() {
        return;
    }
    let dom = query_run.evaluator.subject.dom;
    // A subtree none of whose elements carries every attribute name each match carries holds no match.
    let attribute_names = query_run.query.required_attribute_name_bits(dom);
    let next = |node| optional_node(unsafe { (dom.next_element_in_subtree)(node, root, attribute_names) });
    let mut candidate = next(root);
    while let Some(element) = candidate {
        if query_run.matches(element) && unsafe { found(context, element) } {
            return;
        }
        candidate = next(element);
    }
}
