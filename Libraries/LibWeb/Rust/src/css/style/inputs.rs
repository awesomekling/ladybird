/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use smallvec::SmallVec;

use super::*;

#[derive(Default)]
pub(crate) struct SubstitutionAttributeSnapshot<'a> {
    pub text: Vec<(&'a [u16], &'a [u16])>,
    pub names_are_ascii_case_insensitive: bool,
}

unsafe extern "C" {
    fn web_css_custom_property_data_reference(data: *const std::ffi::c_void);
    fn web_css_custom_property_data_unreference(data: *const std::ffi::c_void);
}

/// The custom-property environment one element holds, kept by the engine so that a row inheriting
/// custom properties answers from here instead of walking the flat tree to the element and reading
/// its environment off it.
///
/// The row is written where the element's environment is installed, which is the only place it
/// moves, and released when the node is retired.
pub(crate) struct RetainedCustomPropertyData {
    data: crate::css::host_shared::HostShared<std::ffi::c_void>,
    /// The store the environment holds its values in, which lives as long as it does; null where
    /// the host did not name it.
    store: crate::css::host_shared::HostShared<std::ffi::c_void>,
}

impl RetainedCustomPropertyData {
    /// # Safety
    /// `data` must be a live `Web::CSS::CustomPropertyData`, and `store` null or its store.
    unsafe fn retain(data: *const std::ffi::c_void, store: *const std::ffi::c_void) -> Self {
        unsafe { web_css_custom_property_data_reference(data) };
        Self {
            data: crate::css::host_shared::HostShared::new(data),
            store: crate::css::host_shared::HostShared::new(store),
        }
    }

    pub(crate) fn data(&self) -> *const std::ffi::c_void {
        self.data.as_ptr()
    }

    /// Another reference to the same environment, for another element that holds it.
    pub(crate) fn share(&self) -> Self {
        // SAFETY: This row keeps the environment live.
        unsafe { Self::retain(self.data(), self.store.as_ptr()) }
    }
}

/// The custom-property environment an element or one of its pseudo-elements holds. The engine
/// names it by identity; the host's object for it is kept when the host installed it, while one
/// the engine moved the element to is an environment the engine resolved, which the host views
/// from its store.
pub(crate) struct HeldCustomPropertyEnvironment {
    pub(crate) identity: u64,
    /// Whether it is the element's animation overlay, over the environment its style resolves to.
    pub(crate) is_animation_overlay: bool,
    /// Whether it declares custom properties of its own, over the environment it inherits.
    pub(crate) declares: bool,
    pub(crate) data: Option<RetainedCustomPropertyData>,
    /// For an animation overlay, the environment it was composed over, which it keeps alive.
    pub(crate) animation_base: Option<AnimationBaseEnvironment>,
}

/// The environment an animation-sampled one was composed over: its identity, its store and the
/// host's environment object, which the sampled one keeps alive.
pub(crate) struct AnimationBaseEnvironment {
    environment: u64,
    store: crate::css::host_shared::HostShared<std::ffi::c_void>,
    data: crate::css::host_shared::HostShared<std::ffi::c_void>,
}

impl AnimationBaseEnvironment {
    pub(crate) fn environment(&self) -> u64 {
        self.environment
    }

    /// One the engine resolved, which the host holds no object of its own for.
    pub(crate) fn resolved_by_engine(environment: u64, store: *const std::ffi::c_void) -> Self {
        Self {
            environment,
            store: crate::css::host_shared::HostShared::new(store),
            data: crate::css::host_shared::HostShared::new(std::ptr::null()),
        }
    }
}

impl Drop for RetainedCustomPropertyData {
    fn drop(&mut self) {
        assert_eq!(
            crate::stage_thread::acting_thread(),
            std::thread::current().id(),
            "a custom-property environment must be released on the document thread"
        );
        // SAFETY: The row owns exactly one reference, taken in `retain`.
        unsafe { web_css_custom_property_data_unreference(self.data.as_ptr()) };
    }
}

impl StyleEngine {
    pub(crate) fn install_layout_style_snapshots(
        &mut self,
        snapshots: std::sync::Arc<crate::layout::style_snapshot::LayoutStyleSnapshotStore>,
    ) {
        self.retained.layout_style_snapshots = snapshots;
    }

    /// The packed sibling count and index the tree-counting functions resolve against, for a caller
    /// outside the longhand drive's frozen inputs. Zero for an identity the retained tree does not
    /// hold.
    #[must_use]
    pub(crate) fn element_tree_counting_inputs(&self, node: StyleNodeID) -> u64 {
        self.retained.element_tree_counting_inputs(node)
    }
}

/// A selector reads the value text of an attribute name, as
/// [`RetainedState::attribute_value_text_readers`] answers.
pub const ATTRIBUTE_VALUE_TEXT_READ_BY_SELECTORS: u32 = 1;
/// An `attr()` can read the value text of an attribute name.
pub const ATTRIBUTE_VALUE_TEXT_READ_BY_ATTR: u32 = 2;

impl RetainedState {
    pub(crate) fn element_tree_counting_inputs(&self, node: StyleNodeID) -> u64 {
        if !self.tree().is_live(node) {
            return 0;
        }
        let Some(parent) = self
            .tree()
            .parent(node)
            .filter(|parent| self.tree().tree_scope(*parent) == self.tree().tree_scope(node))
        else {
            return (1_u64 << 32) | 1;
        };
        let mut count = 0_u64;
        let mut index = 0_u64;
        for child in self.tree().dom_children(parent).filter(|child| !child.is_text()) {
            count += 1;
            if child == node {
                index = count;
            }
        }
        (count << 32) | index
    }

    /// Keep the custom-property environment an element now holds. A null `data` records that the
    /// element holds none.
    ///
    /// # Safety
    /// `data` must be null or a live `Web::CSS::CustomPropertyData` carrying `store`, and, where it
    /// is an animation overlay, keep alive the environment `animation_base` names; `animation_base`
    /// is `None` for any other environment.
    pub(crate) unsafe fn set_element_custom_property_data(
        &mut self,
        node: StyleNodeID,
        data: *const std::ffi::c_void,
        store: *const std::ffi::c_void,
        environment: u64,
        declares: bool,
        animation_base: Option<(u64, *const std::ffi::c_void, *const std::ffi::c_void)>,
    ) {
        if data.is_null() {
            self.element_custom_property_data.remove(&node);
            return;
        }
        if let Some(existing) = self.element_custom_property_data.get(&node)
            && existing.data.as_ref().is_some_and(|existing| existing.data() == data)
        {
            return;
        }
        if environment != 0 {
            self.computed_group_sets
                .set_node_custom_property_environment(node, environment);
            // A descendant substitutes under the environment it inherits from this element, such
            // as the one an animation samples custom properties into.
            if self.custom_property_environments.store(environment).is_none() {
                unsafe { self.custom_property_environments.retain(environment, store) };
            }
        }
        self.element_custom_property_data.insert(
            node,
            HeldCustomPropertyEnvironment {
                identity: environment,
                is_animation_overlay: animation_base.is_some(),
                declares,
                animation_base: animation_base.map(|(environment, store, data)| AnimationBaseEnvironment {
                    environment,
                    store: crate::css::host_shared::HostShared::new(store),
                    data: crate::css::host_shared::HostShared::new(data),
                }),
                data: Some(unsafe { RetainedCustomPropertyData::retain(data, store) }),
            },
        );
    }

    /// The identity, the store and the host object of the environment the one an element holds was
    /// composed over, where the element's animations sampled custom properties into it: `(0, null,
    /// null)` for one composed over none, and `None` for any other environment.
    pub(crate) fn element_custom_property_animation_base(
        &self,
        node: StyleNodeID,
    ) -> Option<(u64, *const std::ffi::c_void, *const std::ffi::c_void)> {
        self.element_custom_property_data
            .get(&node)?
            .animation_base
            .as_ref()
            .map(|base| (base.environment, base.store.as_ptr(), base.data.as_ptr()))
    }

    /// The environment an element holds, as it was last kept: the host's object for it, or null with
    /// the identity of one the engine resolved. The host keeps no copy of its own.
    pub(crate) fn element_custom_property_data(&self, node: StyleNodeID) -> (*const std::ffi::c_void, u64) {
        Self::held_environment_answer(self.element_custom_property_data.get(&node))
    }

    fn held_environment_answer(held: Option<&HeldCustomPropertyEnvironment>) -> (*const std::ffi::c_void, u64) {
        held.map_or((std::ptr::null(), 0), |held| {
            (
                held.data
                    .as_ref()
                    .map_or(std::ptr::null(), RetainedCustomPropertyData::data),
                held.identity,
            )
        })
    }

    /// Keep the custom-property environment one of an element's synthetic pseudo-elements now holds;
    /// a null `data` is one holding none.
    ///
    /// Unlike an element's, a pseudo-element's `declares` is whether what its own style resolves to
    /// is not simply the environment its originating element passes on, which the host tells.
    ///
    /// # Safety
    /// `data` must be null or a live `Web::CSS::CustomPropertyData`, and `store` null or its store.
    #[expect(
        clippy::too_many_arguments,
        reason = "the environment and the one an overlay is over, as the element's"
    )]
    pub(crate) unsafe fn set_pseudo_element_custom_property_data(
        &mut self,
        node: StyleNodeID,
        pseudo: u8,
        data: *const std::ffi::c_void,
        store: *const std::ffi::c_void,
        environment: u64,
        declares_own: bool,
        animation_base: Option<(u64, *const std::ffi::c_void, *const std::ffi::c_void)>,
    ) {
        if data.is_null() {
            self.pseudo_element_custom_property_data.remove(&(node, pseudo));
            return;
        }
        if self
            .pseudo_element_custom_property_data
            .get(&(node, pseudo))
            .and_then(|existing| existing.data.as_ref())
            .is_some_and(|existing| existing.data() == data)
        {
            return;
        }
        self.pseudo_element_custom_property_data.insert(
            (node, pseudo),
            HeldCustomPropertyEnvironment {
                identity: environment,
                is_animation_overlay: animation_base.is_some(),
                declares: declares_own,
                data: Some(unsafe { RetainedCustomPropertyData::retain(data, store) }),
                animation_base: animation_base.map(|(environment, store, data)| AnimationBaseEnvironment {
                    environment,
                    store: crate::css::host_shared::HostShared::new(store),
                    data: crate::css::host_shared::HostShared::new(data),
                }),
            },
        );
    }

    /// The kinds of the element's synthetic pseudo-elements that hold a custom-property
    /// environment, one bit per kind.
    pub(crate) fn pseudo_elements_with_custom_property_data(&self, node: StyleNodeID) -> u64 {
        if self.pseudo_element_custom_property_data.is_empty() {
            return 0;
        }
        (0..u64::BITS as u8)
            .filter(|&pseudo| self.pseudo_element_custom_property_data.contains_key(&(node, pseudo)))
            .fold(0, |kinds, pseudo| kinds | (1 << pseudo))
    }

    /// The environment one of an element's synthetic pseudo-elements holds, as
    /// `element_custom_property_data` answers for the element.
    pub(crate) fn pseudo_element_custom_property_data(
        &self,
        node: StyleNodeID,
        pseudo: u8,
    ) -> (*const std::ffi::c_void, u64) {
        Self::held_environment_answer(self.pseudo_element_custom_property_data.get(&(node, pseudo)))
    }

    /// What a sample of one of an element's synthetic pseudo-elements reads of the environment it
    /// holds: its store, the identity and the store of the one its own style resolved to beneath
    /// what its animations composed, and whether that one declares custom properties of its own.
    /// Nulls for a pseudo-element holding none, and `None` where the engine has no store for it.
    pub(crate) fn pseudo_element_custom_property_sample_inputs(
        &self,
        node: StyleNodeID,
        pseudo: u8,
    ) -> Option<(*const std::ffi::c_void, u64, *const std::ffi::c_void, bool)> {
        let Some(held) = self.pseudo_element_custom_property_data.get(&(node, pseudo)) else {
            return Some((std::ptr::null(), 0, std::ptr::null(), false));
        };
        let store = held
            .data
            .as_ref()
            .map(|data| data.store.as_ptr())
            .filter(|store| !store.is_null())
            .or_else(|| self.custom_property_environments.store(held.identity))?;
        let (base_environment, base_store) = held
            .animation_base
            .as_ref()
            .map_or((held.identity, store), |base| (base.environment, base.store.as_ptr()));
        Some((store, base_environment, base_store, held.declares))
    }

    /// Note that the element's style reads what a moved custom-property environment can change other
    /// than through `var()`, so a move computes it again rather than handing it the moved one.
    pub(crate) fn note_element_recomputes_on_environment_move(&mut self, node: StyleNodeID) {
        self.environment_move_recompute_nodes.insert(node);
    }

    pub(crate) fn element_recomputes_on_environment_move(&self, node: StyleNodeID) -> bool {
        self.environment_move_recompute_nodes.contains(&node)
    }

    pub fn set_sampled_composition_identity(&mut self, node: StyleNodeID, record: u64) {
        self.computed_group_sets.set_sampled_composition_identity(node, record);
    }

    /// Refresh the container-query projection at the computed-record publication funnel. The
    /// projection owns the name spellings, so a sealed evaluator never has to follow an AK string
    /// or a computed-group payload.
    pub fn set_element_container_query_inputs(&mut self, node: StyleNodeID, style_record: u64) {
        let complete_record = self
            .computed_group_sets
            .style_record_payloads(style_record)
            .is_some_and(|payloads| payloads.len() > crate::css::computed_value_types::STYLE_GROUP_INDEX_BOX);
        if !complete_record {
            self.container_query_inputs.clear(node);
            return;
        }
        let Some(payloads) = self.computed_group_sets.style_record_payloads(style_record) else {
            self.container_query_inputs.clear(node);
            return;
        };
        let values = crate::css::computed_value_views::ComputedValuesView::new(
            crate::css::host_shared::SharedPayload::as_pointer_slice(payloads),
        );
        let box_values = values.box_values();
        let names = box_values
            .container_name
            .raws()
            .iter()
            .map(|raw| match unsafe { ak::utf16_string_units(raw) } {
                ak::Utf16StringUnits::Ascii(units) => units.iter().copied().map(u16::from).collect(),
                ak::Utf16StringUnits::Utf16(units) => units.to_vec(),
            })
            .collect();
        self.container_query_inputs.set(
            node,
            tree::ContainerQueryInputRow {
                style_record,
                names,
                is_size_container: box_values.is_size_container,
                is_inline_size_container: box_values.is_inline_size_container,
                is_scroll_state_container: box_values.is_scroll_state_container,
                writing_mode: values.writing_mode(),
                direction: values.direction(),
            },
        );
    }

    pub(super) fn container_query_inputs(&self, node: StyleNodeID) -> Option<&tree::ContainerQueryInputRow> {
        self.container_query_inputs.get(node)
    }
}

/// What an element's published style record says about the box it asks for. The layout tree build
/// reads this for an element that has no box yet, where the arena has nothing to answer from.
#[derive(Clone, Copy)]
pub struct PublishedBoxFacts {
    pub display: crate::css::display::FfiDisplay,
    pub content_visibility: u8,
    pub position: u8,
    pub float_: u8,
    /// Whether the record suppresses the element's native appearance, which decides whether an
    /// input's native widget box is built at all.
    pub appearance_is_none: bool,
}

/// What the style mirror says about the element a text node's box takes its style from: the text's
/// flat-tree parent, which is the slot it is assigned to or else its DOM parent. A text under a
/// shadow root or the document has no element above it and answers with every field cleared.
#[derive(Clone, Copy, Default)]
pub struct TextStyleParentFacts {
    pub has_style_parent: bool,
    pub parent_display_is_contents: bool,
    pub parent_collapses_whitespace: bool,
    pub style_record: u64,
}

/// What the style mirror publishes about a text node's characters. The characters are shared with
/// the document rather than copied; the language tag is the one the text node's DOM parent element
/// resolves to, and is resolved only where the row's transform reads one, since a tag the document
/// never consulted must not reach the rendering key.
#[derive(Default)]
pub struct PublishedTextSource {
    pub data: ak::Utf16String,
    pub locale: Option<Vec<u16>>,
    pub is_password_input: bool,
}

/// What a published record says about the content a box is generated from.
pub struct PublishedContentFacts {
    pub counters_are_none: bool,
    pub content_is_keyword: bool,
    pub content_is_strings_only: bool,
}

impl RetainedState {
    pub(super) fn push_pending_region(&mut self, regions: &mut Vec<ImpactRegion>, region: ImpactRegion) {
        let before = regions.capacity();
        regions.push(region);
        let after = regions.capacity();
        self.memory.reserve_required(
            MemoryCategory::BatchScratch,
            ((after - before) * size_of::<ImpactRegion>()) as u64,
        );
    }

    /// Says whether the document matches id and class selectors ASCII case-insensitively.
    ///
    /// A document only learns its mode while it is still empty - a parser reads it from the doctype
    /// before the first element arrives, and `document.open()` has already removed every element and
    /// every sheet by the time it resets it - so no rule compiled against the other folding and no
    /// fact published under it can outlive the change.
    pub fn set_fold_id_and_class_name_case(&mut self, fold: bool) {
        self.fold_id_and_class_name_case = fold;
    }

    /// The HTML namespace, for an HTML document, or none for any other kind. It is what decides
    /// whether an attribute name from the legacy list compares its value case-insensitively.
    pub fn set_html_element_namespace(&mut self, namespace: StyleAtomID) {
        self.html_element_namespace = namespace;
    }

    #[must_use]
    pub fn tree(&self) -> &StyleNodeTree {
        &self.tree
    }

    #[must_use]
    pub fn connected_element_count(&self) -> u32 {
        self.tree.connected_element_count()
    }

    #[must_use]
    #[cfg(test)]
    pub(super) fn program(&self) -> &StyleSheetProgram {
        &self.program
    }

    pub(super) fn add_routing_rule(&mut self, rule: RuleID, program: SelectorProgramID) {
        self.programs.settle_memory(&mut self.memory);
        // A detached sheet's routes were shed, and reattachment restores the current routes of
        // every live rule in the sheet, so routes added for a rule edited while its sheet is
        // detached would come back twice. The exclusion covers the edit until the sheet reattaches.
        if self
            .sheets_excluded_from_routing
            .contains(self.program.rule_sheet(rule).0 as usize)
        {
            return;
        }
        let routing = Arc::get_mut(&mut self.routing).expect("routing program is shared outside a planning epoch");
        routing.add_rule(rule, program, &self.programs);
    }

    /// The process-global atom for one interned name identity.
    ///
    /// Selector names and DOM facts intern through here and nowhere else. Two tables keyed by the
    /// same word but assigning their own sequences would compare unequal for the same name, which
    /// is a silent failure to match rather than a loud one.
    pub fn intern_atom(&mut self, raw: usize) -> StyleAtomID {
        self.atoms.intern_cpp_raw(raw)
    }

    /// Take into the document an atom the host acquired for one of its names.
    pub fn adopt_atom(&mut self, raw: usize, atom: StyleAtomID) {
        self.atoms.adopt_cpp_raw(raw, atom);
    }

    /// Keep the atom a render-side publication names live for as long as the publication does.
    ///
    /// A published SVG reference names an id that may name no element at all, and an atom nothing
    /// answers to has no other owner: a sweep would reclaim it and hand its number to the next
    /// name interned, which the publication would then read as the element it points at.
    pub fn retain_published_atom(&mut self, atom: StyleAtomID) {
        self.atoms.retain_published(atom);
    }

    /// Give up the retention `retain_published_atom` took, as a publication is cleared or replaced.
    pub fn release_published_atom(&mut self, atom: StyleAtomID) {
        self.atoms.release_published(atom);
    }

    /// The document-local atom for a name qualified by a namespace.
    ///
    /// `[ns|x]` names an attribute that `[x]` does not, and one element can carry both. So the
    /// qualified form is a name of its own: the attribute is published under it as well as under
    /// its local name, and the selector that names a namespace tests only this one.
    pub fn intern_qualified_atom(&mut self, namespace: StyleAtomID, name: StyleAtomID) -> StyleAtomID {
        self.atoms.intern_qualified(namespace, name)
    }

    /// Takes the qualified atom the host acquired for `namespace` and `name`; see
    /// `style_engine_acquire_host_qualified_atom`.
    pub fn adopt_qualified_atom(&mut self, namespace: StyleAtomID, name: StyleAtomID, atom: StyleAtomID) {
        self.atoms.adopt_qualified(namespace, name, atom);
    }

    /// Record what a custom property's name atom spells, and the fly string it is.
    ///
    /// # Safety
    /// `raw` must be zero or a live `AK::Utf16FlyString` raw representation.
    pub unsafe fn note_custom_property_name(
        &mut self,
        name: StyleAtomID,
        raw: usize,
        text: &[u16],
        counters: &mut Counters,
    ) {
        if unsafe { self.custom_property_environments.note_name(name, raw, text) } {
            counters.bump(Counter::CustomPropertyNamesPublished);
        }
    }

    // A stable declaration owner changes contents without changing its address. Reserve zero for
    // absence and one for the initial block, then issue fresh identities for subsequent edits.
    pub(crate) fn next_declaration_block_version(&mut self) -> u32 {
        self.declaration_block_version = self
            .declaration_block_version
            .checked_add(1)
            .expect("declaration revision overflow");
        self.declaration_block_version
    }

    pub(super) fn compile_selectors(
        &mut self,
        selectors: &[&CompiledSelector],
        namespaces: NamespaceScope,
        scope: &ScopeChain<'_>,
        reusable: Option<SelectorProgramID>,
        counters: &mut Counters,
    ) -> SelectorProgramID {
        let fold_id_and_class_name_case = self.fold_id_and_class_name_case;
        let html_element_namespace = self.html_element_namespace;
        let atoms = &mut self.atoms;
        let mut intern = |raw: usize, namespace: Option<StyleAtomID>| -> StyleAtomID {
            let local = atoms.intern_raw(raw);
            let Some(namespace) = namespace else {
                return local;
            };
            atoms.intern_qualified(namespace, local)
        };

        let mut compiler = SelectorCompiler::new(
            &mut intern,
            fold_id_and_class_name_case,
            html_element_namespace,
            namespaces,
        );
        for selector in selectors {
            let compiled = compiler.compile_in_scope(selector, scope);
            match compiled.marker {
                Some(marker) => {
                    if let Some(counter) = marker.counter() {
                        counters.bump(counter);
                    }
                    counters.bump(Counter::ExactSelectorEntries);
                }
                None => counters.bump(Counter::ExactSelectorEntries),
            }
        }
        let compiled = compiler.finish();
        let mut requirements_changed = false;
        for name in compiled.attribute_value_text_names() {
            requirements_changed |= self.attribute_value_text_names.insert(name);
        }
        if requirements_changed {
            self.attribute_value_text_requirements_version += 1;
        }
        if let Some(reusable) = reusable
            && self.programs.get(reusable) == &compiled
        {
            return reusable;
        }
        let program = self.programs.add(compiled);
        self.selector_programs_need_sweep |= reusable.is_some();
        self.programs.settle_memory(&mut self.memory);
        program
    }

    pub(crate) fn compile_selector_query(&mut self, selectors: &[&CompiledSelector]) -> SelectorProgram {
        let fold_id_and_class_name_case = self.fold_id_and_class_name_case;
        let html_element_namespace = self.html_element_namespace;
        let atoms = &mut self.atoms;
        let mut intern = |raw: usize, namespace: Option<StyleAtomID>| -> StyleAtomID {
            let local = atoms.intern_raw(raw);
            let Some(namespace) = namespace else {
                return local;
            };
            atoms.intern_qualified(namespace, local)
        };

        let mut compiler = SelectorCompiler::new(
            &mut intern,
            fold_id_and_class_name_case,
            html_element_namespace,
            NamespaceScope::default(),
        );
        for selector in selectors {
            compiler.compile_for_query(selector);
        }
        let program = compiler.finish();
        let mut requirements_changed = false;
        for name in program.attribute_value_text_names() {
            requirements_changed |= self.attribute_value_text_names.insert(name);
        }
        if requirements_changed {
            self.attribute_value_text_requirements_version += 1;
        }
        program
    }

    /// Moves whenever a name comes to require its value text: a selector's here, or an `attr()`'s
    /// anywhere in the process.
    pub fn attribute_value_text_requirements_version(&self) -> u64 {
        self.attribute_value_text_requirements_version
            .wrapping_add(crate::css::parser::arbitrary_substitution::attr_names_read_generation())
    }

    /// Which readers of an attribute name's value text there are, as `ATTRIBUTE_VALUE_TEXT_READ_BY_*`
    /// bits: a selector whose operator an atom cannot answer, and an `attr()`. The host records the
    /// text of a value under this name only if there is one.
    #[must_use]
    pub fn attribute_value_text_readers(&self, name: StyleAtomID) -> u32 {
        let mut readers = 0;
        if self
            .facts
            .attribute_name_keys(name)
            .any(|key| self.attribute_value_text_names.contains(&key))
        {
            readers |= ATTRIBUTE_VALUE_TEXT_READ_BY_SELECTORS;
        }
        if self.attr_may_read_attribute(name) {
            readers |= ATTRIBUTE_VALUE_TEXT_READ_BY_ATTR;
        }
        readers
    }

    /// Substitution reads only attributes in no namespace, by local name.
    fn attr_may_read_attribute(&self, name: StyleAtomID) -> bool {
        self.facts.attribute_name_has_no_namespace(name)
            && self
                .facts
                .attribute_name_text(name)
                .is_some_and(crate::css::parser::arbitrary_substitution::attr_may_read_name)
    }

    #[must_use]
    pub fn memory(&self) -> &MemoryController {
        &self.memory
    }

    /// Whether the last transaction planned nothing but derived child reactions, read once.
    pub fn take_only_derived_child_reactions(&mut self) -> bool {
        std::mem::take(&mut self.last_transaction_only_derived_child_reactions)
    }

    /// Apply an accepted input to the authoritative fact arrangement.
    ///
    /// A fact exists in the store exactly because a mutation published it, which is what lets the
    /// evaluator distinguish "this element has no such class" from "nothing ever told me about this
    /// element".
    pub(super) fn apply_to_facts_without_settling(&mut self, key: InputKey, new: InputValue) {
        match (key, new) {
            (InputKey::LocalFeature(node, feature), InputValue::Feature(value)) => match feature {
                LocalFeatureKey::TagName => {
                    if let FeatureValue::Atom(atom) = value {
                        self.facts.set_tag(node, atom, &mut self.memory);
                    }
                }
                LocalFeatureKey::PartExposure => self.facts.set_part_exposure(
                    node,
                    match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    },
                ),
                LocalFeatureKey::Language => self.facts.set_language(
                    node,
                    match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    },
                ),
                LocalFeatureKey::Directionality => self.facts.set_directionality(
                    node,
                    match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    },
                    &mut self.memory,
                ),
                LocalFeatureKey::HeadingLevel => self.facts.set_heading_level(
                    node,
                    match value {
                        FeatureValue::Number(level) => level as u8,
                        _ => 0,
                    },
                ),
                LocalFeatureKey::FoldedTagName => self.facts.set_folded_tag(
                    node,
                    match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    },
                    &mut self.memory,
                ),
                // Parts and custom states are published as complete sets after their individual
                // journal deltas have been recorded. Arrival is only a routing key.
                LocalFeatureKey::Part(_) | LocalFeatureKey::CustomState(_) | LocalFeatureKey::ArrivingFacts => {}
                // A text node is not a style node, so nothing in the tree can say it is there.
                // `Present` on this key means the element is empty.
                LocalFeatureKey::Emptiness => self.facts.set_has_text_content(node, !value.holds()),
                LocalFeatureKey::Id => self.facts.set_id(
                    node,
                    match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    },
                    &mut self.memory,
                ),
                LocalFeatureKey::Class(class) => self.facts.set_class(node, class, value.holds(), &mut self.memory),
                LocalFeatureKey::Attribute(name) => {
                    // The value's atom rides on the same delta. Presence is what routing reads; the
                    // value is what an exact test compares, and a cold pass has no DOM to ask.
                    let atom = match value {
                        FeatureValue::Atom(atom) => atom,
                        _ => StyleAtomID::NONE,
                    };
                    self.facts
                        .set_attribute(node, name, atom, value.holds(), &mut self.memory);
                }
            },
            (InputKey::State(node, fact), InputValue::State(value)) => {
                self.facts.set_state(node, fact, value, &mut self.memory);
            }
            _ => {}
        }
    }

    /// The routing keys of every fact an arriving element announced, read back off the element.
    ///
    /// A fact is in the store by the time routing runs, so this is what the individual inputs would
    /// have published between them - a class each, the tag, the id, each attribute name.
    #[must_use]
    pub(super) fn routing_keys_of_arriving_facts(&self, node: StyleNodeID) -> SmallVec<[RoutingKey; 4]> {
        let mut keys = SmallVec::new();

        let tag = self.facts.tag_of_node(node);
        if !tag.is_none() {
            keys.push(RoutingKey::TagName(tag));
        }
        let folded_tag = self.facts.folded_tag_of_node(node);
        if !folded_tag.is_none() && folded_tag != tag {
            keys.push(RoutingKey::TagName(folded_tag));
        }
        let id = self.facts.id_of_node(node);
        if !id.is_none() {
            keys.push(RoutingKey::Id(id));
        }
        for &class in self.facts.classes_of_node(node) {
            keys.push(RoutingKey::Class(class));
        }
        for attribute in self.facts.attributes_of_node(node) {
            for key in self.facts.attribute_name_keys(attribute) {
                keys.push(RoutingKey::AttributeName(key));
            }
        }
        if !self.facts.language_of(node).is_none() {
            keys.push(RoutingKey::Language);
        }
        let directionality = self.facts.directionality_of(node);
        if !directionality.is_none() {
            keys.push(RoutingKey::Directionality(directionality));
        }
        if let Some(row) = self.facts.primary().row_of(node) {
            for &part in self.facts.primary().parts_of(row) {
                keys.push(RoutingKey::Part(part));
            }
            for &state in self.facts.primary().custom_states_of(row) {
                keys.push(RoutingKey::CustomState(state));
            }
        }
        for fact in self.facts.states_of_node(node).facts() {
            keys.push(RoutingKey::State(fact));
        }
        // An element arrives empty or not, and either way that is a positional truth about it that
        // nothing else in this transaction says.
        keys.push(RoutingKey::Structural);

        keys
    }

    pub(super) fn settled_tree_relations(&self, node: StyleNodeID) -> TreeRelations {
        TreeRelations {
            parent: self.tree.parent(node),
            previous_element_sibling: self.tree.previous_element_sibling(node),
            next_element_sibling: self.tree.next_element_sibling(node),
            tree_scope: self.tree.tree_scope(node),
            assigned_slot: self.tree.assigned_slot_of(node),
        }
    }

    pub(super) fn depth_recompute_nodes(
        &self,
        staged_rows: &[(StyleNodeID, Option<TreeRelations>, Option<TreeRelations>)],
    ) -> HashSet<StyleNodeID> {
        staged_rows
            .iter()
            .filter_map(|&(node, before, relations)| {
                let relations = relations?;
                (before.is_none() || self.tree.parent(node) != relations.parent).then_some(node)
            })
            .collect()
    }

    pub(super) fn link(&mut self, node: StyleNodeID, new: TreeRelations) {
        self.tree.set_parent(node, new.parent);
        self.tree.set_next_element_sibling(node, new.next_element_sibling);
        self.tree
            .set_previous_element_sibling(node, new.previous_element_sibling);
        if let Some(next) = new.next_element_sibling {
            self.tree.set_previous_element_sibling(next, Some(node));
        }
        match new.previous_element_sibling {
            Some(previous) => self.tree.set_next_element_sibling(previous, Some(node)),
            None => {
                if let Some(parent) = new.parent {
                    self.tree.set_first_element_child(parent, Some(node));
                }
            }
        }

        if new.tree_scope != TreeScopeID::DOCUMENT {
            self.tree.enable_tree_scopes(&mut self.memory);
        }
        if self.tree.has_tree_scopes() {
            self.tree.set_tree_scope(node, new.tree_scope);
        }

        // The slot a slottable is assigned to is its parent in the flat tree, and `::slotted()`
        // walks back along it. A slot name change reassigns it with no DOM mutation at all, so the
        // relation has to be carried by the delta rather than derived from the tree.
        self.tree.set_assigned_slot(node, new.assigned_slot, &mut self.memory);
    }

    /// A fact whose value is an atom is absent when the atom is, so the two are one conversion.
    pub(super) fn atom_or_absent(atom: StyleAtomID) -> FeatureValue {
        match atom.is_none() {
            true => FeatureValue::Absent,
            false => FeatureValue::Atom(atom),
        }
    }

    /// Report the element's resolved language tag.
    /// Record the element's namespace.
    ///
    /// An element's namespace is fixed when it is created, so this is a fact the store holds rather
    /// than an input that moves: nothing routes from it, and no journal entry is needed.
    pub fn set_element_namespace(&mut self, node: StyleNodeID, namespace: StyleAtomID) {
        self.facts.set_namespace(node, namespace);
    }

    /// Record the element's heading level, or zero where it has none.
    /// Record that an element is a `<slot>`, which decides whether `::slotted()` can name it.
    pub fn set_element_is_slot(&mut self, node: StyleNodeID, is_slot: bool) {
        self.facts.set_is_slot(node, is_slot);
    }

    /// Replace the element facts the style computation's adjustments read.
    pub fn set_element_adjustment_facts(&mut self, node: StyleNodeID, facts: u32) {
        self.computed_group_sets.set_adjustment_facts(node, facts);
    }

    /// Record the element-reference pseudo kind represented by an internal shadow-tree element.
    pub fn set_element_associated_pseudo_kind(&mut self, node: StyleNodeID, pseudo_kind_plus_one: u8) {
        self.computed_group_sets
            .set_associated_pseudo_kind(node, pseudo_kind_plus_one);
    }

    /// The element facts the store holds, as `bridge::element_adjustment_fact` names them. A text
    /// node and a retired identity hold none.
    #[must_use]
    pub fn element_adjustment_facts(&self, node: StyleNodeID) -> u32 {
        self.computed_group_sets.adjustment_facts(node)
    }

    /// Replace the element facts a layout row built for the element records, and which principal
    /// box the element asks for.
    pub fn set_element_construction_facts(&mut self, node: StyleNodeID, facts: u32, box_kind: u8) {
        self.computed_group_sets.set_construction_facts(node, facts, box_kind);
    }

    /// The element facts the store holds, as `bridge::element_construction_fact` names them. A
    /// retired identity holds none. A text node has no element columns and holds one of the facts
    /// on its own row, since which kind of tree it sits in decides what its row answers about the
    /// text control around it.
    #[must_use]
    pub fn element_construction_facts(&self, node: StyleNodeID) -> u32 {
        if node.is_text() {
            return if self.tree.text_is_in_user_agent_shadow_tree(node) {
                crate::css::style::bridge::element_construction_fact::IS_IN_USER_AGENT_SHADOW_TREE
            } else {
                0
            };
        }
        self.computed_group_sets.construction_facts(node)
    }

    /// Which principal box the element asks for, as the raw byte of a `bridge::ElementBoxKind`.
    /// A text node and a retired identity ask for nothing in particular.
    #[must_use]
    pub fn element_box_kind(&self, node: StyleNodeID) -> u8 {
        self.computed_group_sets.box_kind(node)
    }

    /// Whether the text node's data is nothing but ASCII whitespace.
    #[must_use]
    pub fn text_is_ascii_whitespace(&self, node: StyleNodeID) -> bool {
        self.tree.text_is_ascii_whitespace(node)
    }

    /// Record the text node's whitespace-only state, as its data now spells it.
    pub fn set_text_is_ascii_whitespace(&mut self, node: StyleNodeID, value: bool) {
        self.tree.set_text_is_ascii_whitespace(node, value, &mut self.memory);
    }

    /// Everything a text node's box renders from, taken in one borrow of the mirror.
    #[must_use]
    pub fn published_text_source(&self, node: StyleNodeID, uses_locale: bool) -> PublishedTextSource {
        let Some(data) = self.tree.text_data(node) else {
            return PublishedTextSource::default();
        };
        PublishedTextSource {
            data: data.clone(),
            locale: uses_locale
                .then(|| self.text_language_tag(node).to_vec())
                .filter(|tag| !tag.is_empty()),
            is_password_input: self.tree.text_is_password_input(node),
        }
    }

    /// The element's resolved language tag, empty where it has none.
    #[must_use]
    pub fn element_language_tag(&self, node: StyleNodeID) -> &[u16] {
        self.facts.language_tag_of(node)
    }

    /// The language tag a text node's transform reads: the one its DOM parent element resolves to.
    /// A text node under a shadow root or the document has no element above it and reads none, the
    /// same answer the document gives for a text node whose parent is not an element.
    #[must_use]
    pub fn text_language_tag(&self, node: StyleNodeID) -> &[u16] {
        self.tree
            .text_parent(node)
            .filter(|parent| self.tree.host_of(*parent).is_none() && !self.tree.is_relation_only(*parent))
            .map_or(&[][..], |parent| self.facts.language_tag_of(parent))
    }

    /// Whether the text node holds the value of a password input.
    #[must_use]
    pub fn text_is_password_input(&self, node: StyleNodeID) -> bool {
        self.tree.text_is_password_input(node)
    }

    /// Record whether the text node holds the value of a password input.
    pub fn set_text_is_password_input(&mut self, node: StyleNodeID, value: bool) {
        self.tree.set_text_is_password_input(node, value, &mut self.memory);
    }

    /// The characters the text node holds.
    #[must_use]
    pub fn text_data(&self, node: StyleNodeID) -> Option<&ak::Utf16String> {
        self.tree.text_data(node)
    }

    /// Record the characters the text node now holds.
    pub fn set_text_data(&mut self, node: StyleNodeID, data: ak::Utf16String) {
        self.tree.set_text_data(node, data, &mut self.memory);
    }

    /// Record which kind of tree the text node arrived in.
    pub fn set_text_is_in_user_agent_shadow_tree(&mut self, node: StyleNodeID, value: bool) {
        self.tree
            .set_text_is_in_user_agent_shadow_tree(node, value, &mut self.memory);
    }

    /// Record whether the list owner's items were renumbered without its layout tree being rebuilt.
    pub fn set_list_owner_has_stale_item_counters(&mut self, node: StyleNodeID, value: bool) {
        self.tree
            .set_list_owner_has_stale_item_counters(node, value, &mut self.memory);
    }

    /// Let go of the stale list owners `forget` answers yes for, which it is asked with the tree.
    pub fn forget_list_owners_with_stale_item_counters(
        &mut self,
        mut forget: impl FnMut(&StyleNodeTree, StyleNodeID) -> bool,
    ) {
        let forgotten: Vec<StyleNodeID> = self
            .tree
            .list_owners_with_stale_item_counters()
            .iter()
            .copied()
            .filter(|&owner| forget(&self.tree, owner))
            .collect();
        for owner in forgotten {
            self.tree
                .set_list_owner_has_stale_item_counters(owner, false, &mut self.memory);
        }
    }

    /// The box facts the element's published style record holds. `None` while the element has no
    /// record: a text node, a retired identity, or an element style has not reached yet.
    #[must_use]
    pub fn element_published_box_facts(&self, node: StyleNodeID) -> Option<PublishedBoxFacts> {
        self.published_box_facts(self.computed_group_sets.assigned_style_record(node))
    }

    /// The style record the mirror published for the element, with the payload pointer a row is
    /// built from. `None` while the element has no record: a retired identity, or a style that has
    /// not reached the element yet.
    #[must_use]
    pub fn element_published_style_record(&self, node: StyleNodeID) -> Option<(u64, *const std::ffi::c_void)> {
        let record = self.computed_group_sets.assigned_style_record(node)?.raw();
        let payloads = self.computed_group_sets.style_record_payloads(record)?;
        Some((record, payloads.as_ptr().cast()))
    }

    /// The published style record of one of the element's pseudo-elements, as
    /// [`Self::element_published_style_record`] answers for the element itself. `None` while the
    /// element styles no such pseudo-element.
    pub fn pseudo_published_style_record(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<(u64, *const std::ffi::c_void)> {
        let record = self.computed_group_sets.pseudo_style_record(node, pseudo_kind)?.raw();
        let payloads = self.computed_group_sets.style_record_payloads(record)?;
        Some((record, payloads.as_ptr().cast()))
    }

    /// The box facts the element's published record for one pseudo-element kind holds. `None`
    /// while the element styles no such pseudo-element.
    #[must_use]
    pub fn pseudo_published_box_facts(&self, node: StyleNodeID, pseudo_kind: u8) -> Option<PublishedBoxFacts> {
        self.published_box_facts(self.computed_group_sets.pseudo_style_record(node, pseudo_kind))
    }

    /// Whether the element's published style record counts a counter down from its own last item,
    /// which nothing short of a full rebuild can renumber.
    #[must_use]
    pub fn element_counter_reset_has_reversed_counter(&self, node: StyleNodeID) -> bool {
        self.published_style_record_view(self.computed_group_sets.assigned_style_record(node))
            .is_some_and(crate::css::computed_value_views::ComputedValuesView::counter_reset_has_reversed_counter)
    }

    /// Whether the element's published style record replaces its contents with a single image.
    #[must_use]
    pub fn element_content_is_single_image(&self, node: StyleNodeID) -> bool {
        self.published_style_record_view(self.computed_group_sets.assigned_style_record(node))
            .is_some_and(crate::css::computed_value_views::ComputedValuesView::content_is_single_image)
    }

    /// What the element's published record for one pseudo-element kind says about its generated
    /// content. `None` while the element styles no such pseudo-element.
    #[must_use]
    pub fn pseudo_published_content_facts(&self, node: StyleNodeID, pseudo_kind: u8) -> Option<PublishedContentFacts> {
        let view = self.published_style_record_view(self.computed_group_sets.pseudo_style_record(node, pseudo_kind))?;
        Some(PublishedContentFacts {
            counters_are_none: view.counter_properties_are_none(),
            content_is_keyword: view.content_is_keyword(),
            content_is_strings_only: view.content_is_strings_only(),
        })
    }

    /// The element's published style record, or its record for one pseudo-element kind, as a view.
    /// `None` while there is no such record.
    #[must_use]
    pub(crate) fn published_style_view(
        &self,
        node: StyleNodeID,
        pseudo_kind: Option<u8>,
    ) -> Option<crate::css::computed_value_views::ComputedValuesView<'_>> {
        let style_record = match pseudo_kind {
            Some(pseudo_kind) => self.computed_group_sets.pseudo_style_record(node, pseudo_kind),
            None => self.computed_group_sets.assigned_style_record(node),
        };
        self.published_style_record_view(style_record)
    }

    /// The dependency flags of the element's published style record, or of its record for one
    /// pseudo-element kind. `None` while there is no such record.
    #[must_use]
    pub(crate) fn published_style_dependency_flags(&self, node: StyleNodeID, pseudo_kind: Option<u8>) -> Option<u8> {
        let style_record = match pseudo_kind {
            Some(pseudo_kind) => self.computed_group_sets.pseudo_style_record(node, pseudo_kind),
            None => self.computed_group_sets.assigned_style_record(node),
        }?;
        self.computed_group_sets
            .style_record_dependency_flags(style_record.raw())
    }

    /// A bit per pseudo-element kind the element holds a settled record for, which is what says
    /// each of them exists at all. The style store settles no record for `::backdrop` or a
    /// highlight pseudo-element, so neither is ever named here.
    #[must_use]
    pub fn published_pseudo_record_mask(&self, node: StyleNodeID) -> u32 {
        self.computed_group_sets.published_pseudo_record_mask(node)
    }

    /// Whether the element is a `<slot>`, whose children the flat tree takes elsewhere.
    #[must_use]
    pub fn element_is_slot(&self, node: StyleNodeID) -> bool {
        self.facts.is_slot(node)
    }

    /// What the text node's flat-tree parent publishes, for the anonymous inline wrapper a text
    /// under a `display: contents` element needs.
    #[must_use]
    pub fn text_style_parent_facts(&self, node: StyleNodeID) -> TextStyleParentFacts {
        let parent = self
            .tree
            .assigned_slot_of(node)
            .or_else(|| self.tree.text_parent(node))
            .filter(|parent| self.tree.host_of(*parent).is_none() && !self.tree.is_relation_only(*parent));
        let Some(parent) = parent else {
            return TextStyleParentFacts::default();
        };
        let style_record = self.computed_group_sets.assigned_style_record(parent);
        let Some(view) = self.published_style_record_view(style_record) else {
            return TextStyleParentFacts::default();
        };
        TextStyleParentFacts {
            has_style_parent: true,
            parent_display_is_contents: view.display().is_contents(),
            parent_collapses_whitespace: view.white_space_collapse()
                == crate::css::css_enums::white_space_collapse::COLLAPSE,
            style_record: style_record.map_or(0, computed::FinalStyleRecordID::raw),
        }
    }

    fn published_box_facts(&self, style_record: Option<computed::FinalStyleRecordID>) -> Option<PublishedBoxFacts> {
        let view = self.published_style_record_view(style_record)?;
        Some(PublishedBoxFacts {
            display: view.display(),
            content_visibility: view.content_visibility(),
            position: view.position(),
            float_: view.float_(),
            appearance_is_none: view.appearance() == crate::css::css_enums::appearance::NONE,
        })
    }

    pub(super) fn published_style_record_view(
        &self,
        style_record: Option<computed::FinalStyleRecordID>,
    ) -> Option<crate::css::computed_value_views::ComputedValuesView<'_>> {
        let payloads = self.computed_group_sets.style_record_payloads(style_record?.raw())?;
        Some(crate::css::computed_value_views::ComputedValuesView::new(
            crate::css::host_shared::SharedPayload::as_pointer_slice(payloads),
        ))
    }

    /// Record what an attribute-value atom spells, for the operators an atom cannot answer and for
    /// `attr()` substitution.
    pub fn set_attribute_value_text(&mut self, value: StyleAtomID, text: &[u16]) {
        self.set_attribute_value_text_with_selector_effect(value, text, true);
    }

    /// As [`Self::set_attribute_value_text`], for a value of a name a selector reads or not. Only
    /// text a selector reads moves the selector plans.
    pub fn set_attribute_value_text_with_selector_effect(
        &mut self,
        value: StyleAtomID,
        text: &[u16],
        read_by_selectors: bool,
    ) {
        self.facts.set_attribute_value_text(value, text, read_by_selectors);
    }

    #[must_use]
    pub fn has_attribute_value_text(&self, value: StyleAtomID) -> bool {
        self.facts.has_attribute_value_text(value)
    }

    /// Record what a language atom spells, so `:lang()` can compare its ranges against the tag.
    pub fn set_element_language_text(&mut self, language: StyleAtomID, text: &[u16], counters: &mut Counters) {
        counters.bump(Counter::LanguageTextsPublished);
        self.facts.set_language_text(language, text);
    }

    /// See `ElementFactStore::note_attribute_name_forms`.
    pub fn note_attribute_name(
        &mut self,
        name: StyleAtomID,
        forms: index::AttributeNameForms,
        local_name: &[u16],
        has_no_namespace: bool,
    ) {
        self.facts
            .note_attribute_name(name, forms, local_name, has_no_namespace);
    }

    #[cfg(test)]
    pub fn note_attribute_name_forms(&mut self, name: StyleAtomID, forms: index::AttributeNameForms) {
        self.facts.note_attribute_name_forms(name, forms);
    }

    /// Record the id an element answers to, or clear it with atom zero.
    pub fn set_element_id_name(&mut self, node: StyleNodeID, name: StyleAtomID) {
        self.tree.set_element_id_name(node, name, &mut self.memory);
    }

    /// Record the unique node id the document names the element by. It arrives with the identity
    /// and never changes while the element holds it.
    pub fn set_element_unique_node_id(&mut self, node: StyleNodeID, unique_node_id: i64) {
        self.tree.set_unique_node_id(node, unique_node_id, &mut self.memory);
    }

    /// The unique node id the document names the element by, or zero for anything else.
    #[must_use]
    pub fn element_unique_node_id(&self, node: StyleNodeID) -> i64 {
        self.tree.unique_node_id(node)
    }

    /// What a row built for the node is painted and hit-tested with.
    #[must_use]
    pub fn node_dom_paint_facts(&self, node: StyleNodeID) -> u8 {
        self.tree.dom_paint_facts(node)
    }

    pub fn set_node_dom_paint_facts(&mut self, node: StyleNodeID, facts: u8) {
        self.tree.set_dom_paint_facts(node, facts, &mut self.memory);
    }

    /// The spans a row built for the element takes from its attributes.
    #[must_use]
    pub fn element_table_spans(&self, node: StyleNodeID) -> super::tree::TableSpans {
        self.tree.table_spans(node)
    }

    pub fn set_element_table_spans(&mut self, node: StyleNodeID, spans: super::tree::TableSpans) {
        self.tree.set_table_spans(node, spans, &mut self.memory);
    }

    pub fn element_replaced_content_input(&self, node: StyleNodeID) -> super::tree::ReplacedContentInput {
        self.tree.replaced_content_input(node)
    }

    pub fn set_element_replaced_content_input(&mut self, node: StyleNodeID, input: super::tree::ReplacedContentInput) {
        self.tree.set_replaced_content_input(node, input, &mut self.memory);
    }

    /// The first element in tree order that answers to `name` inside `tree_scope`.
    #[must_use]
    pub fn element_by_id(&self, tree_scope: TreeScopeID, name: StyleAtomID) -> Option<StyleNodeID> {
        self.tree.element_by_id(tree_scope, name)
    }

    /// The style group payloads the element's published record holds, for a reader that reaches an
    /// element by identity rather than through a layout row. `None` while the element has no
    /// record.
    #[must_use]
    pub fn element_published_style_payloads(&self, node: StyleNodeID) -> Option<&[*const std::ffi::c_void]> {
        let record = self.computed_group_sets.assigned_style_record(node)?;
        let payloads = self.computed_group_sets.style_record_payloads(record.raw())?;
        Some(crate::css::host_shared::SharedPayload::as_pointer_slice(payloads))
    }

    pub fn set_shadow_root(&mut self, host: StyleNodeID, shadow_root: StyleNodeID) {
        self.tree.set_shadow_root(host, shadow_root, &mut self.memory);
    }

    /// Replace the ordered list of nodes a slot has assigned to it, text nodes included.
    pub fn set_slot_assigned_nodes(&mut self, slot: StyleNodeID, nodes: &[StyleNodeID]) {
        self.tree.set_assigned_nodes(slot, nodes, &mut self.memory);
    }

    /// Replace the document's top layer, in membership order.
    pub fn set_top_layer_elements(&mut self, elements: &[StyleNodeID]) {
        self.top_layer_elements.clear();
        self.top_layer_elements.extend_from_slice(elements);
    }

    #[must_use]
    pub fn top_layer_elements(&self) -> &[StyleNodeID] {
        &self.top_layer_elements
    }

    // -- DOM child sequence ------------------------------------------------------------------
    //
    // Text nodes take identities so that the style tree can describe the DOM child sequence, but
    // nothing selects, styles or invalidates them. Their arrivals and departures cross as host fact
    // writes and are spliced in the order the host made them, rather than journaled.

    /// Retire text identities as their nodes disconnect.
    pub fn retire_text_style_nodes(&mut self, nodes: &[StyleNodeID]) {
        self.layout_style_snapshots.retire(nodes);
        self.tree.retire_texts(nodes, &mut self.memory);
    }

    /// Splice nodes into the DOM child sequence, given as `(node, parent, previous sibling)`
    /// triples of raw identities in tree order, so that each previous sibling is linked first.
    pub fn link_style_nodes_in_dom_order(&mut self, links: &[u32]) {
        for &[node, parent, previous] in links.as_chunks::<3>().0 {
            let Some(node) = StyleNodeID::from_raw(node) else {
                continue;
            };
            self.tree
                .link_in_dom_order(node, StyleNodeID::from_raw(parent), StyleNodeID::from_raw(previous));
        }
    }

    pub fn unlink_style_node_from_dom_order(&mut self, node: StyleNodeID, parent: Option<StyleNodeID>) {
        self.tree.unlink_from_dom_order(node, parent);
    }

    /// Mark an identity that stands in the tree only to be named by relations. The document is one:
    /// it owns the DOM child sequence its children hang from, and it is never styled or matched.
    pub fn mark_relation_only_style_node(&mut self, node: StyleNodeID) {
        self.tree.mark_relation_only(node);
    }

    // -- Stylesheet program ------------------------------------------------------------------
    //
    // Every CSSOM mutation maps to a precise typed delta. None of them produces a generic document
    // invalidation, and the granularity is what makes that true: a declaration edit journals the
    // declaration field alone, so selector truth is untouched by an edit that cannot change it.

    pub fn add_sheet(&mut self, object: StyleSheetObjectID, origin: CascadeOrigin) -> SheetID {
        // Reserving an unattached sheet identity cannot change any scope's semantic program.
        let sheet = self.program.add_sheet(object, origin);
        self.settle_program();
        sheet
    }

    /// A sheet whose routes were shed while it was detached contributes routes again the moment
    /// it reattaches. The registry must be whole before the attachment's transaction plans, so
    /// this runs at recording time rather than waiting for the next sweep.
    pub(super) fn restore_routing_for_reattached_sheet(&mut self, sheet: SheetID) {
        if !self.sheets_excluded_from_routing.set(sheet.0 as usize, false).0 {
            return;
        }
        let rules = self
            .program
            .rules_in_sheet(sheet)
            .into_iter()
            .filter(|&rule| self.program.rule_is_live(rule))
            .filter_map(|rule| Some((rule, self.program.rule_version(rule).selector_program?)));
        let programs = &self.programs;
        let routing = Arc::get_mut(&mut self.routing).expect("routing program is shared outside a planning epoch");
        for (rule, program) in rules {
            routing.add_rule(rule, program, programs);
        }
        routing.settle_memory(&mut self.memory);
    }

    /// Record the animation names an element's computed style references.
    ///
    /// This is an index, not an input: an element whose `animation-name` changed was already
    /// recomputed by whatever changed it. What the index is for is the other direction - a
    /// `@keyframes` rule finding the elements running the animation it describes.
    pub fn set_element_animation_names(&mut self, node: StyleNodeID, names: &[StyleAtomID]) {
        self.facts.set_animation_names(node, names, &mut self.memory);
    }

    /// Record the names of the CSS animations the host holds for one of an element's animation
    /// lists, in the order it holds them. The names arrive packed into one buffer because a list is
    /// almost always a single name, and a length per name is cheaper than a handle per name.
    pub fn set_element_css_defined_animations(
        &mut self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
        name_lengths: &[u32],
        name_units: &[u16],
        definition_words: &[u64],
    ) {
        let mut names = Vec::with_capacity(name_lengths.len());
        let mut offset = 0usize;
        for &length in name_lengths {
            let length = length as usize;
            let end = offset + length;
            assert!(end <= name_units.len(), "animation name lengths overrun their buffer");
            names.push(crate::css::css_string::CssString::from_utf16(&name_units[offset..end]));
            offset = end;
        }
        assert!(
            definition_words.len() == name_lengths.len() * animations::APPLIED_DEFINITION_WORD_COUNT,
            "every published animation name must come with its applied definition"
        );
        let definitions = definition_words
            .as_chunks::<{ animations::APPLIED_DEFINITION_WORD_COUNT }>()
            .0
            .iter()
            .map(|words| animations::AppliedAnimationDefinition::from_words(words))
            .collect::<Vec<_>>();
        let keyframes_generation = self.animation_keyframes.generation();
        self.css_defined_animations.set(
            node,
            slot,
            names.into_boxed_slice(),
            definitions.into_boxed_slice(),
            keyframes_generation,
        );
    }

    /// The names of the CSS animations the host holds for one of an element's animation lists.
    #[must_use]
    pub(crate) fn element_css_defined_animations(
        &self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
    ) -> &[crate::css::css_string::CssString] {
        self.css_defined_animations.names(node, slot)
    }

    /// Record the `@keyframes` one style scope defines, as the host's rule cache for that scope
    /// resolved them. Published at the style update's begin boundary, before any element's
    /// animation definitions are matched against them.
    ///
    /// # Safety
    /// Every declaration's `value` must be a live style value the host holds a reference to for the
    /// duration of the call.
    pub unsafe fn set_tree_scope_animation_keyframes(
        &mut self,
        tree_scope: TreeScopeID,
        shadow_root_identity: usize,
        name_lengths: &[u32],
        name_units: &[u16],
        published_buffers: animations::PublishedEffectBuffers<'_>,
    ) {
        unsafe {
            self.animation_keyframes.set(
                tree_scope,
                shadow_root_identity,
                name_lengths,
                name_units,
                published_buffers,
            );
        }
        // The host drops the sets the scope published before, so a plan still owed names what the
        // table publishes now.
        for plan in self
            .nodes_owing_animation_definitions
            .values_mut()
            .chain(self.animation_definitions_being_applied.as_mut())
        {
            plan.resolve_keyframes_again(&self.animation_keyframes);
        }
    }

    /// The `@keyframes` the document's style scopes define.
    #[must_use]
    pub(crate) fn animation_keyframes(&self) -> &animations::AnimationKeyframes {
        &self.animation_keyframes
    }

    /// Record the timing of the animations the host holds for one of an element's animation lists.
    ///
    /// The times travel as raw `f64` bits beside a word of presence and kind flags, eight times and
    /// two words per animation, because a row is a handful of scalars and a struct per animation
    /// would cost more than the scalars do. A `linear()` easing's stops travel the same way, in a
    /// buffer shared by the list, which each row names its own part of by range.
    pub fn set_element_animation_timing_rows(
        &mut self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
        words: &[u32],
        times: &[u64],
        linear_points: &[u64],
    ) {
        let times = times.iter().map(|&bits| f64::from_bits(bits)).collect::<Vec<_>>();
        let linear_points = linear_points
            .iter()
            .map(|&bits| f64::from_bits(bits))
            .collect::<Vec<_>>();
        self.animation_timing_rows
            .set(node, slot, words, &times, &linear_points);
    }

    /// Describe the effects the host holds for one of an element's animation lists, in composite
    /// order, so the animation stage can build its own batch instead of walking the host's keyframe
    /// sets. Published beside the timing rows, at the same funnels.
    ///
    /// # Safety
    /// Every declaration's value must be a live style value for the duration of the call.
    pub unsafe fn set_element_animation_effect_descriptions(
        &mut self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
        published_buffers: animations::PublishedEffectBuffers<'_>,
    ) {
        unsafe {
            self.animation_effect_descriptions.set(node, slot, published_buffers);
        }
    }

    /// Lend out the effects the host described for one of an element's animation lists; see
    /// `AnimationEffectDescriptions::take`.
    pub(crate) fn element_has_animation_effect_descriptions(
        &self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
    ) -> bool {
        self.animation_effect_descriptions.contains(node, slot)
    }

    pub(crate) fn take_element_animation_effect_descriptions(
        &mut self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
    ) -> Option<Box<[animations::PublishedEffect]>> {
        self.animation_effect_descriptions.take(node, slot)
    }

    pub(crate) fn restore_element_animation_effect_descriptions(
        &mut self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
        effects: Box<[animations::PublishedEffect]>,
    ) {
        self.animation_effect_descriptions.restore(node, slot, effects);
    }

    /// The document's base URL, as the host last published it.
    #[must_use]
    pub(crate) fn document_base_url(&self) -> &[u8] {
        &self.document_resource_contexts.document_base_url
    }

    /// The registry `custom_property_registry` answers from, shared, for a caller that also needs
    /// the engine mutably while it asks.
    #[must_use]
    pub(crate) fn shared_custom_property_registry(
        &self,
    ) -> std::sync::Arc<crate::css::custom_properties::CustomPropertyRegistry> {
        self.custom_property_registry.clone()
    }

    /// The timing rows of one of an element's animation lists, in composite order.
    #[must_use]
    pub(crate) fn element_animation_timing_rows(
        &self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
    ) -> &[animations::AnimationTimingRow] {
        self.animation_timing_rows.rows(node, slot)
    }

    /// The `linear()` stops the rows of one of an element's animation lists name by range.
    #[must_use]
    pub(crate) fn element_animation_timing_row_linear_points(
        &self,
        node: StyleNodeID,
        slot: animations::AnimationSlot,
    ) -> &[crate::css::easing::FfiLinearEasingPoint] {
        self.animation_timing_rows.linear_points(node, slot)
    }

    /// The elements the engine knows to be animated: those with animations or transitions the host
    /// published, or animations a pass decided on. A pass samples these against the transform
    /// reference boxes the last layout committed.
    pub(crate) fn animated_nodes(&self) -> impl Iterator<Item = StyleNodeID> + '_ {
        self.animation_timing_rows
            .nodes()
            .chain(self.animation_effect_descriptions.nodes())
            .chain(self.css_defined_animations.nodes())
            .chain(self.element_transitions.nodes())
            .chain(self.nodes_owing_animation_definitions.keys().map(|(node, _)| *node))
            .chain(self.nodes_owing_an_animation_sample.iter().copied())
            .chain(self.nodes_owing_a_transition_registration.keys().copied())
    }

    /// Record the current time each of the document's animation timelines was sampled at. Published
    /// whole at the style update's begin boundary, since a timeline's time only moves outside one.
    pub fn set_animation_timeline_samples(&mut self, identities: &[u32], words: &[u32], times: &[u64]) {
        let times = times.iter().map(|&bits| f64::from_bits(bits)).collect::<Vec<_>>();
        self.animation_timeline_samples.set(identities, words, &times);
    }

    #[must_use]
    pub(crate) fn animation_timeline_samples(&self) -> &animations::AnimationTimelineSamples {
        &self.animation_timeline_samples
    }

    /// Record the font metrics a `rem` resolves against: the host's defaults before the document
    /// element has style, and then the ones of each record the host installs on it, which a style
    /// update can cross.
    pub fn set_root_element_font_metrics(&mut self, words: &[u64], depends_on_viewport_metrics: bool) {
        self.root_element_font_metrics =
            animations::RootElementFontMetrics::from_words(words, depends_on_viewport_metrics);
    }

    /// Record the custom properties an element declares or references. Also an index rather than an
    /// input, and for the same reason: it answers which elements an `@property` registration reaches.
    pub fn set_element_custom_property_names(
        &mut self,
        node: StyleNodeID,
        environment: u64,
        name_atoms: &[u32],
        uses_unnamed: bool,
        uses_custom_functions: bool,
        counters: &mut Counters,
    ) {
        self.facts
            .set_environment_custom_property_names(node, environment, name_atoms, &mut self.memory, counters);
        self.facts
            .set_uses_unnamed_custom_properties(node, uses_unnamed, &mut self.memory);
        self.facts
            .set_uses_custom_functions(node, uses_custom_functions, &mut self.memory);
    }

    pub fn set_tree_scope_root(&mut self, tree_scope: TreeScopeID, root: StyleNodeID) {
        if tree_scope == TreeScopeID::DOCUMENT {
            return;
        }
        let index = tree_scope.0 as usize;
        if let Some(previous_root) = self.scope_roots.get(index).copied().flatten()
            && previous_root != root
        {
            self.scope_by_root.remove(previous_root);
        }
        self.scope_roots.insert(index, Some(root));
        self.scope_by_root.insert(root, tree_scope);
        // The root is a node selectors reach and nothing publishes features for, so this is where it
        // gets a row of its own.
        self.facts.ensure_row(root);
    }

    pub(super) fn scope_root(&self, tree_scope: TreeScopeID) -> Option<StyleNodeID> {
        self.scope_roots.get(tree_scope.0 as usize).copied().flatten()
    }

    /// Everything a sheet attached to these scopes can decide.
    ///
    /// A scope-local sheet reaches the tree it is attached to, and out of it only through `:host`,
    /// `::slotted()` and `::part()` - which reach the host and the nodes slotted into it. So a sheet
    /// in a shadow root can be bounded even when its rules dispatch on nothing enumerable, where the
    /// document's own scope has no bound narrower than the document.
    /// Where a sheet decides, taking in both the scopes it is attached to now and the ones it was
    /// attached to when the transaction began.
    pub(super) fn scopes_of_sheet(
        &self,
        sheet: SheetID,
        departed_sheet_scopes: &[(SheetID, TreeScopeID)],
    ) -> Vec<TreeScopeID> {
        let mut scopes = self.program.sheet_scopes(sheet);
        for &(departed_sheet, scope) in departed_sheet_scopes {
            if departed_sheet == sheet && !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        scopes
    }

    pub(super) fn regions_reachable_from_scopes(
        &self,
        scopes: &[TreeScopeID],
        subject_leaves_scope: bool,
        host_is_a_subject: bool,
    ) -> Option<Vec<ImpactRegion>> {
        if scopes.is_empty() {
            return None;
        }
        let mut regions = Vec::new();
        for &scope in scopes {
            let root = match self.scope_root(scope) {
                Some(root) => root,
                // The document's own scope records no root and has no bound narrower than itself,
                // which is a different thing from a shadow scope that has not named one yet.
                None if scope == TreeScopeID::DOCUMENT => return None,
                // A shadow scope whose root has never been named holds no element the engine knows
                // about: every element in a shadow tree takes its place in the style tree through
                // that root, so a scope that has not named one has nothing inside it for a sheet
                // attached there to decide. Attaching a sheet to a shadow root numbers its scope
                // without populating it, which is what `attachShadow` immediately followed by
                // `adoptedStyleSheets` does, and answering that with the document restyles the page
                // once per component.
                //
                // A program that leaves its scope reaches the host instead, so it needs the root to
                // find one, and a scope with no root names no host either.
                None if !subject_leaves_scope => continue,
                None => continue,
            };
            match subject_leaves_scope {
                // `:host` and `::slotted()` describe the host and what is slotted into it, which are
                // in the host's own tree and not in the one the sheet is attached to. A rule using
                // them says nothing about the shadow tree, so naming it would be the widest part of
                // the answer for the narrowest reason.
                // A root with no host is a shadow tree whose host has not taken its place in the
                // style tree, so the tree hangs off nothing the engine can reach and the rule
                // decides for no element that exists. The host computes its style from scratch when
                // it does arrive, which is what makes saying so safe rather than optimistic.
                true => match self.tree.host_of(root) {
                    Some(host) => {
                        if host_is_a_subject {
                            regions.push(ImpactRegion::Node(host));
                        }
                        regions.push(ImpactRegion::StrictSubtree(host));
                    }
                    None => continue,
                },
                false => regions.push(ImpactRegion::Subtree(root)),
            }
        }
        Some(regions)
    }

    /// Everything a named rule in these scopes can reach through its consumers.
    ///
    /// Unlike a selector program, a named rule has no one subject position: an `@keyframes` or
    /// `@property` rule can be referenced both inside its shadow tree and by a declaration matching
    /// `:host` or `::slotted()`. The consumer index identifies the exact nodes when it is complete;
    /// these regions bound that index, and are also the conservative answer when it is not.
    pub(super) fn regions_reachable_for_named_consumers(&self, scopes: &[TreeScopeID]) -> Option<Vec<ImpactRegion>> {
        let mut regions = self.regions_reachable_from_scopes(scopes, false, false)?;
        let outside_regions = self.regions_reachable_from_scopes(scopes, true, true)?;
        regions.extend(outside_regions);
        Some(regions)
    }

    /// Where an entry with no dispatch key can have subjects, when its own shape still says.
    ///
    /// Three things narrow an entry the dispatch buckets gave up on. `:root` matches the document's
    /// root element and nothing else, whatever else its compound tests. A child combinator names
    /// what the subject's parent must be, which does have postings to enumerate - one lookup per
    /// parent candidate reaches every subject the entry can have, where the alternative is every
    /// element in the document. And a descendant combinator says the same thing more weakly: the
    /// subject is somewhere under a named ancestor rather than directly beneath a named parent.
    ///
    /// A relative query the subject has to satisfy is the fourth: `:has(.error)` names no feature of
    /// its own, but it can only match an anchor of that query, and the witness compound does have a
    /// posting to enumerate.
    ///
    /// `None` means the entry says nothing, and the caller falls back to the scope. Every answer here
    /// is a superset of the true subject set, which is what a plan is allowed to be.
    pub(super) fn regions_from_subject_position(
        &self,
        compiled: &SelectorProgram,
        entry: usize,
        document_root: StyleNodeID,
        bounding_scopes: Option<&[TreeScopeID]>,
    ) -> Option<Vec<ImpactRegion>> {
        if compiled.subject_is_only_the_root(entry) {
            return Some(vec![ImpactRegion::Node(document_root)]);
        }

        // The sets below are disjunctions: the constrained relative is reachable by at least one of
        // the keys, so the union over all of them covers it. An empty set is no constraint at all.
        //
        // A parent is the tightest, so it is asked for first; an ancestor is consulted when there is
        // no child combinator, and a relative query only when the subject's own position says nothing.
        let (keys, region): (_, fn(StyleNodeID) -> ImpactRegion) = match compiled.subject_parent_dispatch(entry) {
            keys if !keys.is_empty() => (keys, ImpactRegion::Children),
            _ => match compiled.subject_ancestor_dispatch(entry) {
                keys if !keys.is_empty() => (keys, ImpactRegion::StrictSubtree),
                _ => match compiled.subject_relative_anchor(entry) {
                    Some((axis, keys)) if !keys.is_empty() => (keys, anchor_region_for(axis)?),
                    _ => return None,
                },
            },
        };
        if keys.is_empty() {
            return None;
        }
        let mut regions = Vec::new();
        for key in keys {
            if !key.has_selector_posting() {
                return None;
            }
            match self.facts.postings().lookup(key) {
                Lookup::Known(posting) => {
                    for relative in posting.candidates() {
                        if bounding_scopes
                            .is_some_and(|scopes| scopes.binary_search(&self.tree.tree_scope(relative)).is_err())
                        {
                            continue;
                        }
                        regions.push(region(relative));
                    }
                }
                Lookup::KnownAbsent => {}
                Lookup::Missing(_) => return None,
            }
        }
        Some(regions)
    }

    /// Record that a rule sits in a cascade layer.
    ///
    /// Said as the rule is compiled rather than as an input: which layer a rule is in is part of what
    /// the rule is, and a rule that moves between layers is recompiled.
    /// Record which longhand properties one of an element's own declarations covers.
    ///
    /// An element-attached declaration is a cascade component above layers: a style attribute beats
    /// every layered and unlayered rule in its context, whatever layer they are in.
    #[allow(clippy::too_many_arguments)]
    pub fn set_element_declared_properties(
        &mut self,
        node: StyleNodeID,
        kind: ElementDeclarationKind,
        declared: &[DeclaredProperty],
        written_values: Vec<RetainedStyleValueData>,
        custom_declarations: Vec<CustomDeclaration>,
        custom_written_values: Vec<RetainedStyleValueData>,
        counters: &mut Counters,
    ) {
        debug_assert!(custom_declarations.is_empty() || kind == ElementDeclarationKind::InlineStyle);
        if matches!(
            kind,
            ElementDeclarationKind::PresentationalHint | ElementDeclarationKind::SvgPresentationAttribute
        ) {
            verify_cascade_winners(self, |_| {
                let mut properties: Vec<u16> = declared.iter().map(|declared| declared.property).collect();
                properties.sort_unstable();
                assert!(
                    properties.windows(2).all(|pair| pair[0] != pair[1]),
                    "element-attached declarations repeat a property"
                );
            });
        }
        let (current_declared, current_declarations_are_complete) = self.facts.element_declared_properties(node, kind);
        if current_declared == declared
            && (kind != ElementDeclarationKind::InlineStyle
                || self.facts.element_custom_declarations(node) == custom_declarations.as_slice())
        {
            return;
        }
        let repair_inputs = current_declarations_are_complete
            .then(|| {
                let previous = self
                    .current_winner_groups()
                    .token_for(WinnerGroupKey::current(node, self.program.version()))
                    .sparse()
                    .ok()
                    .map(|(_, state)| state)?;
                let retained = self.current_answer_identity(node)?;
                self.match_answers.answer(retained)?;
                Some((previous, retained, current_declared.to_vec()))
            })
            .flatten();
        self.facts
            .set_element_declared_properties(node, kind, declared.to_vec(), written_values);
        if kind == ElementDeclarationKind::InlineStyle {
            self.facts
                .set_element_custom_declarations(node, custom_declarations, custom_written_values);
        }
        let Some((previous, retained, previous_declared)) = repair_inputs else {
            return;
        };
        let mut changed_properties: Vec<u16> = previous_declared
            .iter()
            .chain(declared)
            .map(|declared| declared.property)
            .collect();
        changed_properties.sort_unstable();
        changed_properties.dedup();
        changed_properties.retain(|&property| {
            previous_declared.iter().find(|declared| declared.property == property)
                != declared.iter().find(|declared| declared.property == property)
        });
        if !changed_properties.is_empty() {
            self.apply_element_declaration_winner_updates(node, previous, retained, &changed_properties, counters);
        }
    }

    /// Repair the exact properties whose element-attached declaration inventory changed.
    ///
    /// Presentational hints are published while the legacy cascade builds its input, after the
    /// transaction has already reused the selector answer. Re-reducing only their changed
    /// properties here keeps the retained top-1 relation current without matching the element.
    pub(super) fn apply_element_declaration_winner_updates(
        &mut self,
        node: StyleNodeID,
        previous: CascadeStateID,
        retained: MatchAnswerID,
        properties: &[u16],
        counters: &mut Counters,
    ) {
        let Some(retained) = self.match_answers.answer(retained) else {
            return;
        };
        let matches = retained
            .iter()
            .copied()
            .filter(|entry| {
                !self.program.declarations_are_complete_for(entry.rule)
                    || self
                        .program
                        .declared_properties_of(entry.rule)
                        .iter()
                        .any(|declared| properties.binary_search(&declared.property).is_ok())
            })
            .map(|entry| entry.materialize(node, &self.programs, 0))
            .collect::<Option<Vec<_>>>();
        let Some(matches) = matches else {
            return;
        };
        counters.add(Counter::ElementDeclarationRepairMatches, matches.len() as u64);
        let Some(updates) = self.exact_cascade_winner_updates_for_properties(node, &matches, None, properties) else {
            return;
        };
        let (state, _) =
            self.with_cascade_interning_counters(|groups| groups.apply_property_updates(previous, &updates), counters);
        let published = if let Some(traversal) = self.batch_matching_traversal.as_mut() {
            traversal.answer_effects.winners.set(
                &mut self.winner_groups,
                node,
                state,
                self.program.version(),
                &mut self.memory,
            )
        } else {
            let mut effects = super::cascade::WinnerEffects::default();
            let published = effects.set(
                &mut self.winner_groups,
                node,
                state,
                self.program.version(),
                &mut self.memory,
            );
            self.install_winner_effects(effects);
            published
        };
        self.winner_groups.settle_memory(&mut self.memory);
        if published {
            counters.bump(Counter::CascadeNodeHandlesPublished);
        }
    }
}

impl StyleEngineState {
    #[must_use]
    pub fn new(device_class: DeviceClass) -> Self {
        Self::new_with_owners(
            device_class,
            DocumentAtoms::for_live_engine(),
            SelectorPrograms::for_live_engine(),
        )
    }

    pub(crate) fn new_for_replay(device_class: DeviceClass) -> Self {
        Self::new_with_owners(
            device_class,
            DocumentAtoms::for_replay(),
            SelectorPrograms::for_replay(),
        )
    }

    fn new_with_owners(device_class: DeviceClass, atoms: DocumentAtoms, programs: SelectorPrograms) -> Self {
        let mut memory = MemoryController::new(device_class);
        let tree = StyleNodeTree::new(&mut memory);
        // Until the host publishes the document's registry, the engine's inputs name an empty one.
        let custom_property_registry =
            std::sync::Arc::new(crate::css::custom_properties::CustomPropertyRegistry::empty());
        Self {
            retained: RetainedState {
                memory,
                admission: AdmissionFacts::default(),
                deferred_pseudo_element: None,
                latent_deferred_pseudo_element: None,
                deferred_pseudo_element_observable_nodes: Vec::new(),
                tree,
                program: StyleSheetProgram::new(),
                native_rules: Default::default(),
                container_effects_for_host: HashMap::default(),
                published_container_verdicts: HashMap::default(),
                container_gates_unheld: HashSet::default(),
                container_input_nodes: HashSet::default(),
                size_container_queries: Default::default(),
                anchor_names: Default::default(),
                declaration_block_version: 1,
                last_transaction_only_derived_child_reactions: false,
                sheets_excluded_from_routing: BitColumn::default(),
                routing_needs_detachment_sweep: false,
                match_workspace: MatchScratch::default(),
                query_match_workspace: MatchScratch::for_selector_query(),
                selector_query_generation: 0,
                query_workspace_generation: 0,
                query_settled_transaction_version: StyleTransactionVersion(0),
                query_sorted_candidates: Vec::new(),
                query_sorted_candidates_stamp: None,
                query_preorder_ranks: HashMap::default(),
                query_preorder_ranks_stamp: None,
                exact_covered_scratch: Vec::new(),
                cascade_compaction_scratch: ordering::CascadeCompactionWorkspace::default(),
                cascade_compaction_scratch_memory: MemoryLease::new(MemoryCategory::BatchScratch),
                top_layer_elements: Vec::new(),
                next_style_transaction_version: StyleTransactionVersion(1),
                document_style_computation_inputs: bridge::FfiDocumentStyleComputationInputs {
                    custom_property_registry: bridge::FfiHostHandle::from_pointer(
                        std::sync::Arc::as_ptr(&custom_property_registry).cast(),
                    ),
                    ..Default::default()
                },
                document_media_snapshot: custom_property_cascade::DocumentMediaSnapshot::default(),
                document_function_snapshot: custom_property_cascade::DocumentFunctionSnapshot::default(),
                driven_viewport: (0.0, 0.0),
                document_resource_contexts: Default::default(),
                custom_property_registry,
                element_custom_property_data: HashMap::default(),
                pseudo_element_custom_property_data: HashMap::default(),
                sampled_custom_property_environments: HashMap::default(),
                sampled_pseudo_element_custom_property_environments: HashMap::default(),
                environment_move_recompute_nodes: HashSet::default(),
                font_resolution: None,
                font_face_snapshot: None,
                font_cascade_memo: None,
                root_font_request: None,
                monospace_font_family: RetainedStyleValueData::from_owned(
                    crate::css::parser::value_parser::value_list(
                        vec![StyleValueData::Keyword {
                            keyword: crate::css::style_compute::keyword::MONOSPACE,
                        }],
                        1,
                        true,
                    ),
                ),
                random_base_values: HashMap::default(),
                random_base_requests: Vec::new(),
                layout_style_snapshots: Default::default(),
                container_query_inputs: Default::default(),
                layer_topology_version: 0,
                sheet_order_version: 0,
                specified_values: SpecifiedValues::new(),
                winner_groups: WinnerGroups::new(),
                computed_group_sets: ComputedGroupSets::default(),
                custom_property_environments: Default::default(),
                nodes_with_substituted_records: HashSet::default(),
                nodes_with_tree_counting_records: HashSet::default(),
                nodes_owing_a_transition_registration: HashMap::default(),
                nodes_owing_explicit_inheritance: HashMap::default(),
                children_explicitly_inherit_marks: HashSet::default(),
                engine_row_child_facts: HashMap::default(),
                batch_pinned_compositions: Vec::new(),
                nodes_owing_an_animation_sample: HashSet::default(),
                pseudo_settles_owed: Default::default(),
                rows_sampled_in_pass: HashMap::default(),
                pseudo_elements_sampled_in_pass: HashMap::default(),
                pseudo_element_environments_named_in_settle: HashMap::default(),
                next_engine_animation_overlay_identity: 0,
                transition_baselines: HashMap::default(),
                element_transitions: Default::default(),
                transition_steps_decided_in_pass: HashMap::default(),
                pseudo_element_transition_steps_decided_in_pass: HashMap::default(),
                taken_transition_step: None,
                counter_style_environment_identities: HashMap::default(),
                nodes_owing_animation_definitions: HashMap::default(),
                animation_definitions_being_applied: None,
                css_defined_animations: Default::default(),
                animation_timing_rows: Default::default(),
                animation_effect_descriptions: Default::default(),
                animation_timeline_samples: Default::default(),
                root_element_font_metrics: Default::default(),
                animation_keyframes: Default::default(),
                custom_property_registrations_changed: false,
                engine_computed_records_pending: HashMap::default(),
                demand_pseudo_records: HashMap::default(),
                flush_stamp: 0,
                style_input_nodes_for_cpp: HashSet::default(),
                tree_counting_input_nodes: HashSet::default(),
                parent_inputs_moved_nodes: HashSet::default(),
                engine_pseudo_record_cache: HashMap::default(),
                batch_answers_complete_but_for_custom_properties: HashMap::default(),
                batch_custom_property_matches: HashMap::default(),
                batch_backing_pseudo_matches: HashMap::default(),
                engine_cold_record_cache: HashMap::default(),
                engine_cold_record_donors: HashMap::default(),
                engine_warm_record_cohorts: HashMap::default(),
                computed_group_set_memory: MemoryLease::new(MemoryCategory::ComputedGroupSet),
                custom_property_environment_memory: MemoryLease::new(MemoryCategory::CustomPropertyEnvironment),
                computed_fixed_metadata_memory: MemoryLease::new(MemoryCategory::ComputedFixedMetadata),
                computed_longhand_table_memory: MemoryLease::new(MemoryCategory::ComputedLonghandTable),
                style_record_memory: MemoryLease::new(MemoryCategory::StyleRecord),
                animation_overlay_memory: MemoryLease::new(MemoryCategory::AnimationOverlayRecord),
                computed_pseudo_assignment_memory: MemoryLease::new(MemoryCategory::ComputedPseudoAssignment),
                style_invalidation_cache: HashMap::default(),
                html_element_namespace: StyleAtomID::NONE,
                match_answers: MatchAnswerCatalog::default(),
                selector_truth_sets: SelectorTruthSetCatalog::default(),
                retained_match_answers: RetainedMatchAnswers::default(),
                retained_selector_incidences: RetainedSelectorIncidences::default(),
                selector_incidence_is_current: false,
                batch_matching_traversal: None,
                completion_exactness: CompletionExactness::Exact,
                route_pruning_states: Mutex::new(RoutePruningStateCache::default()),
                prefix_caches: std::sync::Arc::default(),
                #[cfg(test)]
                force_bounded_prefix_completion: false,
                prepared_batch_matching_traversal: None,
                published_match_answers: PublishedMatchAnswers::default(),
                host_entry_causes: HashMap::default(),
                transaction_fact_view: None,
                facts: ElementFactStore::new(),
                programs,
                attribute_value_text_names: HashSet::default(),
                attribute_value_text_requirements_version: 0,
                selector_programs_need_sweep: false,
                routing: Arc::new(RoutingRegistry::new()),
                selector_truth_changes: SelectorTruthChanges::default(),
                already_planned_selector_truth: DeltaBatch::default(),
                selector_truth_changes_active: false,
                relational_witnesses: RelationalWitnesses::default(),
                pending_witness_effects: Vec::new(),
                witness_effect_scratch: MemoryLease::new(MemoryCategory::BatchScratch),
                relational_witness_residency: MemoryLease::new(MemoryCategory::RetainedWitness),
                scope_roots: Column::default(),
                scope_by_root: SegmentedNodeColumn::default(),
                scope_programs: intern_table::InternTable::default(),
                vacant_scope_programs: Vec::new(),
                scope_dispatch_templates: HashMap::default(),
                scope_cascade_templates: HashMap::default(),
                ancestor_dispatch_templates: HashMap::default(),
                scope_program_by_scope: Column::default(),
                atoms,
                fold_id_and_class_name_case: false,
                #[cfg(test)]
                diagnostic_plan_capture: None,
            },
            host: HostState {
                suspended_style_pass: None,
                update_cold_matching_batch: None,
                font_resolver: None,
                random_state: std::collections::hash_map::RandomState::new(),
                random_serial: 0,
                #[cfg(feature = "style-recording")]
                recording_id: None,
                journal: NormalizationJournal::new(),
                deferred_geometry_journal: NormalizationJournal::new(),
                flushing_deferred_geometry_journal: false,
                deferred_element_style_inputs: Vec::new(),
                latent_deferred_pseudo_element_style_inputs: Vec::new(),
                deferred_element_style_inputs_are_pending: false,
                applied_style_reactions: Vec::new(),
                externally_recorded_style_input_nodes: HashSet::default(),
                held_style_records: HashMap::default(),
                deferred_element_style_input_memory: MemoryLease::new(MemoryCategory::NormalizationJournal),
                initial_tree_batch_applied: false,
                initial_tree_bulk_load_is_pending: false,
                tree_staging: TreeRelationStaging::default(),
                tree_staging_memory: MemoryLease::new(MemoryCategory::NormalizationJournal),
                program_staging: ProgramStaging::default(),
                sheet_occurrences: HashMap::default(),
                sheet_occurrence_storage_bytes: 0,
                sheet_occurrence_memory: MemoryLease::new(MemoryCategory::RuleProgram),
                sheet_rule_replacement: None,
                ffi_style_transaction_output: bridge::FfiStyleTransactionOutput::default(),
                ffi_style_transaction_output_memory: MemoryLease::new(MemoryCategory::BridgeBuffer),
                submitted_style_pass_output: None,
                ffi_style_node_query: Vec::new(),
                ffi_style_node_query_memory: MemoryLease::new(MemoryCategory::BridgeBuffer),
                reclaimed_style_atoms: Vec::new(),
                retired_custom_property_data: Vec::new(),
                environment_moves_in_flight: HashMap::default(),
                style_atoms_swept: false,
                atom_sweep_waits_for_host: false,
                atom_sweep_skipped_by_submitted_pass: false,
                replay_reclaimed_style_atoms: None,
            },
        }
    }

    pub(crate) fn ensure_random_base_value(&mut self, node: StyleNodeID, name: &[u16], element_shared: bool) -> f64 {
        use std::hash::BuildHasher;

        let key = (name.to_vec(), (!element_shared).then_some(node));
        if let Some(value) = self.retained.random_base_values.get(&key) {
            return *value;
        }
        self.host.random_serial = self.host.random_serial.wrapping_add(1);
        let bits = self.host.random_state.hash_one((self.host.random_serial, &key));
        let value = (bits >> 11) as f64 / ((1_u64 << 53) as f64);
        self.retained.random_base_values.insert(key, value);
        value
    }

    pub(crate) fn begin_recording(&mut self, device_class: DeviceClass) {
        #[cfg(feature = "style-recording")]
        {
            self.host.recording_id = record_replay::begin_recording_stream(device_class as u8);
            if self.host.recording_id.is_some() {
                self.retained.memory.enable_recording_policy();
            }
        }
        #[cfg(not(feature = "style-recording"))]
        let _ = device_class;
    }

    pub(crate) fn end_recording(&mut self) {
        #[cfg(feature = "style-recording")]
        {
            record_replay::end_recording_stream(self.host.recording_id.take());
            self.retained.memory.disable_recording_policy();
        }
    }

    pub(crate) fn record_boundary_call(
        &self,
        kind: record_replay::EventKind,
        write_payload: impl FnOnce(&mut record_replay::PayloadWriter),
    ) {
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = kind;
            let _ = write_payload;
        }
        #[cfg(feature = "style-recording")]
        let Some(engine_id) = self.host.recording_id else {
            return;
        };
        #[cfg(feature = "style-recording")]
        record_replay::record_engine_event(engine_id, kind, write_payload);
    }

    pub(crate) fn recording_pointer_token(&self, pointer: usize) -> Option<u64> {
        #[cfg(feature = "style-recording")]
        return self
            .host
            .recording_id
            .map(|engine_id| record_replay::pointer_token(engine_id, pointer));
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = pointer;
            None
        }
    }

    pub(crate) fn recording_atom_pointer_token(&self, pointer: usize) -> Option<u64> {
        #[cfg(feature = "style-recording")]
        return self
            .host
            .recording_id
            .map(|_| record_replay::atom_pointer_token(pointer));
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = pointer;
            None
        }
    }

    pub(crate) fn recording_first_response(&self, category: u8, identity: u64) -> bool {
        #[cfg(feature = "style-recording")]
        return self
            .host
            .recording_id
            .is_some_and(|engine_id| record_replay::first_response(engine_id, category, identity));
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = (category, identity);
            false
        }
    }

    pub(crate) fn recording_id(&self) -> Option<u64> {
        #[cfg(feature = "style-recording")]
        return self.host.recording_id;
        #[cfg(not(feature = "style-recording"))]
        None
    }

    pub(crate) fn forget_recording_atom_mappings(&self, atoms: impl IntoIterator<Item = u32>) {
        #[cfg(feature = "style-recording")]
        if let Some(engine_id) = self.host.recording_id {
            record_replay::forget_atom_mappings(engine_id, atoms);
        }
        #[cfg(not(feature = "style-recording"))]
        let _ = atoms;
    }

    pub(crate) fn recording_atom_mappings(&self) -> RecordedAtomMappings {
        #[cfg(not(feature = "style-recording"))]
        return RecordedAtomMappings {
            atoms: Vec::new(),
            qualified_atoms: Vec::new(),
        };
        #[cfg(feature = "style-recording")]
        {
            let engine_id = self
                .host
                .recording_id
                .expect("only a recording engine serializes atom mappings");
            let mut atoms = self
                .atoms
                .raw()
                .iter()
                .filter(|(_, atom)| record_replay::first_atom_mapping(engine_id, atom.0))
                .map(|(pointer, atom)| {
                    (
                        self.recording_atom_pointer_token(*pointer)
                            .expect("only a recording engine serializes atom mappings"),
                        atom.0,
                    )
                })
                .collect::<Vec<_>>();
            atoms.sort_unstable_by_key(|(_, atom)| *atom);
            let mut qualified_atoms = self
                .atoms
                .qualified()
                .iter()
                .filter(|(_, atom)| record_replay::first_atom_mapping(engine_id, atom.0))
                .map(|((namespace, name), atom)| (*namespace, *name, atom.0))
                .collect::<Vec<_>>();
            qualified_atoms.sort_unstable_by_key(|(_, _, atom)| *atom);
            RecordedAtomMappings { atoms, qualified_atoms }
        }
    }

    /// Record a change to style inputs which are properties of the document environment rather
    /// than of an element or stylesheet rule.
    pub fn record_environment_change(&mut self, counters: &mut Counters) {
        self.discard_prepared_batch_matching_traversal();
        self.host
            .journal
            .record_complete_scope_action(InputKind::Environment, &mut self.retained.memory, counters);
    }

    /// Whether the first tree batch can be installed directly.
    pub(crate) fn can_bulk_load_initial_tree(&self) -> bool {
        !self.host.initial_tree_batch_applied
    }

    /// Install a first batch of unique element arrivals without journalling one structural input
    /// per row. The document root is the transaction envelope, so planning will still choose the
    /// same exact whole-document result at first observation.
    pub(crate) fn bulk_load_initial_tree(
        &mut self,
        document_root: StyleNodeID,
        arrivals: &[(StyleNodeID, TreeRelations)],
        counters: &mut Counters,
    ) {
        debug_assert!(!self.host.initial_tree_batch_applied);
        debug_assert!(!arrivals.is_empty());
        debug_assert!(arrivals.iter().any(|&(node, _)| node == document_root));

        for &(node, relations) in arrivals {
            self.link(node, relations);
        }
        self.publish_budget_inputs();

        let root_relations = arrivals
            .iter()
            .find_map(|&(node, relations)| (node == document_root).then_some(relations))
            .unwrap();
        let recorded = self.record(
            InputKey::TreeRelations(document_root),
            InputValue::TreeRelations(None),
            InputValue::TreeRelations(Some(root_relations)),
            counters,
        );
        debug_assert!(recorded);

        let folded_rows = arrivals.len() - 1;
        counters.add(Counter::RawMutationRecords, folded_rows as u64);
        counters.add(Counter::TreeDeltas, folded_rows as u64);
        counters.bump(Counter::InitialBulkLoads);
        counters.add(Counter::InitialBulkTreeRows, arrivals.len() as u64);
        self.host.initial_tree_batch_applied = true;
        self.host.initial_tree_bulk_load_is_pending = true;
    }

    #[must_use]
    pub fn has_pending_transaction(&self) -> bool {
        (self.host.deferred_element_style_inputs_are_pending && !self.host.deferred_element_style_inputs.is_empty())
            || self.has_applied_style_reactions()
            || !self.host.journal.is_empty()
            || (!self.host.flushing_deferred_geometry_journal && !self.host.deferred_geometry_journal.is_empty())
            || !self.host.tree_staging.is_empty()
            || self.host.program_staging.is_dirty()
            || self.host.sheet_rule_replacement.is_some()
            || self.host.suspended_style_pass.is_some()
            || !self.retained.pseudo_settles_owed.is_empty()
    }

    /// Whether a selector whose answer for an element depends on the element's children (`:has()`,
    /// `:empty`) may take part in the next transaction. Such a selector lets a node anywhere in the
    /// tree decide an element's style, so a program still waiting to be installed counts as one.
    #[must_use]
    pub fn may_have_child_dependent_selectors(&self) -> bool {
        self.retained.programs.may_have_child_dependent_programs()
            || !self.retained.routing.relational_routes().is_empty()
            || self.host.program_staging.is_dirty()
            || self.host.sheet_rule_replacement.is_some()
    }

    #[must_use]
    pub fn has_deferred_geometry_transaction(&self) -> bool {
        !self.host.flushing_deferred_geometry_journal && !self.host.deferred_geometry_journal.is_empty()
    }

    /// Whether any element style input is still deferred, waiting for the first transaction with a
    /// document root. A rootless flush drains the journal but preserves these — so an engine that
    /// reports no pending transaction can still owe an element its recomputation.
    #[must_use]
    pub fn has_deferred_element_style_inputs(&self) -> bool {
        !self.host.deferred_element_style_inputs.is_empty()
    }

    /// Record an exact style reaction for one element, merged with what the element already owes.
    /// It joins the next transaction.
    pub fn record_element_style_input(&mut self, node: StyleNodeID, reaction: u8, inherited_style_groups: u8) {
        if reaction == 0 {
            return;
        }
        // A recorded input asks for the C++ computation, whatever the engine derived for the
        // element beside it. For the style pass accounting, a transaction stays one of derived
        // child reactions when the recorded inputs join reactions the engine derived.
        let derived_already = self.has_deferred_element_style_input(node);
        let externally_recorded_already = self.host.externally_recorded_style_input_nodes.contains(&node);
        self.record_derived_element_style_input(node, reaction, inherited_style_groups);
        if !derived_already || externally_recorded_already {
            self.host.externally_recorded_style_input_nodes.insert(node);
        }
        self.retained.style_input_nodes_for_cpp.insert(node);
    }

    /// A container's measurements or style moved under this dependent's retained answer.
    pub fn record_container_query_input(&mut self, node: StyleNodeID) {
        self.retained.container_input_nodes.insert(node);
        self.record_derived_element_style_input(
            node,
            transaction::STYLE_REACTION_PUBLISHED_STYLE | transaction::STYLE_REACTION_RECOMPUTE_STYLE,
            0,
        );
    }

    /// A hidden SVG descendant loses its host style before the style transaction completes.
    /// Keep that loss as an input and derive the resource's replacement record in the engine.
    pub fn set_element_container_query_inputs(&mut self, node: StyleNodeID, style_record: u64) {
        self.retained.set_element_container_query_inputs(node, style_record);
        if style_record == 0 {
            self.host.held_style_records.remove(&node);
        } else {
            self.host.held_style_records.insert(node, style_record);
            // A `rem` resolves against the font metrics of the record the document element holds.
            if self.retained.computed_group_sets.adjustment_facts(node)
                & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT
                != 0
                && let Some(inputs) = self.retained.root_font_inputs_from_raw_record(style_record)
            {
                self.retained
                    .set_root_element_font_metrics(&inputs.metrics, inputs.depends_on_viewport);
            }
        }
        if style_record == 0
            && self.retained.computed_group_sets.adjustment_facts(node)
                & bridge::element_adjustment_fact::IS_SVG_ELEMENT
                != 0
        {
            self.record_derived_element_style_input(
                node,
                transaction::STYLE_REACTION_PUBLISHED_STYLE | transaction::STYLE_REACTION_RECOMPUTE_STYLE,
                0,
            );
        }
    }

    /// Record a style reaction the engine derived itself for one element, or one C++ derived from
    /// a reaction it applied: the engine settles it where it can.
    pub fn record_derived_element_style_input(&mut self, node: StyleNodeID, reaction: u8, inherited_style_groups: u8) {
        if reaction == 0 {
            return;
        }
        self.defer_element_style_input(node, reaction, inherited_style_groups);
        self.host.deferred_element_style_inputs_are_pending = true;
        self.host.externally_recorded_style_input_nodes.remove(&node);
    }

    pub fn record_tree_counting_style_input(&mut self, node: StyleNodeID) {
        self.record_derived_element_style_input(
            node,
            transaction::STYLE_REACTION_PUBLISHED_STYLE | transaction::STYLE_REACTION_RECOMPUTE_STYLE,
            0,
        );
        self.retained.tree_counting_input_nodes.insert(node);
    }

    /// Fold the style input an element owes into the reaction C++ is about to apply to it, when
    /// that reaction covers it: a materialization covers anything, while a record delta covers
    /// only what it already carries. The folded input is consumed; one not covered stays owed to
    /// the next transaction. Returns the merged reaction in the low byte and the merged inherited
    /// style groups in the next, or zero when nothing was folded.
    pub fn absorb_element_style_input(
        &mut self,
        node: StyleNodeID,
        reaction: u8,
        inherited_style_groups: u8,
        absorbs_any: bool,
    ) -> u32 {
        let Ok(index) = self
            .host
            .deferred_element_style_inputs
            .binary_search_by_key(&InputKey::ElementStyleInput(node), |pending| pending.key)
        else {
            return 0;
        };
        let InputValue::ElementStyleInput {
            reaction: pending_reaction,
            inherited_style_groups: pending_inherited_style_groups,
        } = self.host.deferred_element_style_inputs[index].new
        else {
            unreachable!();
        };
        if !absorbs_any
            && (pending_reaction & !reaction != 0 || pending_inherited_style_groups & !inherited_style_groups != 0)
        {
            return 0;
        }
        self.host.deferred_element_style_inputs.remove(index);
        self.host.externally_recorded_style_input_nodes.remove(&node);
        u32::from(reaction | pending_reaction)
            | (u32::from(inherited_style_groups | pending_inherited_style_groups) << 8)
    }

    /// Drop the style input an element owes: a record computed for the element answers it. What
    /// the input asks of the element's children is not answered by the element's own record: an
    /// ancestor becoming visible reveals children that were never styled, and a descendant
    /// recomputation reaches past the element. That part stays owed, so that the element's next
    /// reaction carries it on to its children.
    pub fn consume_element_style_input(&mut self, node: StyleNodeID) {
        if let Ok(index) = self
            .host
            .deferred_element_style_inputs
            .binary_search_by_key(&InputKey::ElementStyleInput(node), |pending| pending.key)
        {
            const CHILD_DIRECTED_REACTIONS: u8 = transaction::STYLE_REACTION_ANCESTOR_BECAME_VISIBLE
                | transaction::STYLE_REACTION_RECOMPUTE_DESCENDANT_STYLES;
            let pending = &mut self.host.deferred_element_style_inputs[index];
            let InputValue::ElementStyleInput { reaction, .. } = pending.new else {
                unreachable!();
            };
            if reaction & CHILD_DIRECTED_REACTIONS != 0 {
                pending.new = InputValue::ElementStyleInput {
                    reaction: reaction & CHILD_DIRECTED_REACTIONS,
                    inherited_style_groups: 0,
                };
            } else {
                self.host.deferred_element_style_inputs.remove(index);
            }
        }
        self.host.externally_recorded_style_input_nodes.remove(&node);
    }

    /// Whether one element still owes a deferred style input, asked per node the way the recorded
    /// batch is. The deferred inputs are kept sorted by key, so this is a binary search.
    #[must_use]
    pub fn has_deferred_element_style_input(&self, node: StyleNodeID) -> bool {
        self.host
            .deferred_element_style_inputs
            .binary_search_by_key(&InputKey::ElementStyleInput(node), |pending| pending.key)
            .is_ok()
    }

    /// Whether settling the pending selector inputs can change geometry derived from the committed
    /// layout. This is deliberately a proof of independence rather than a list of properties which
    /// usually avoid layout: anything not explicitly known to preserve geometry remains observable.
    #[must_use]
    pub fn pending_transaction_may_affect_layout_geometry(&self) -> bool {
        if self.host.journal.is_empty() {
            return !self.host.tree_staging.is_empty()
                || self.host.program_staging.is_dirty()
                || self.host.sheet_rule_replacement.is_some()
                || !self.host.deferred_element_style_inputs.is_empty()
                || self.host.initial_tree_bulk_load_is_pending;
        }
        if !self.host.journal.markers().is_empty()
            || !self.host.tree_staging.is_empty()
            || self.host.program_staging.is_dirty()
            || self.host.sheet_rule_replacement.is_some()
            || !self.host.deferred_element_style_inputs.is_empty()
            || self.host.initial_tree_bulk_load_is_pending
        {
            return true;
        }

        let mut checked_keys = HashSet::default();
        self.host.journal.inputs().any(|input| {
            let keys = match input.key {
                InputKey::LocalFeature(_, LocalFeatureKey::PartExposure | LocalFeatureKey::ArrivingFacts) => {
                    return true;
                }
                InputKey::LocalFeature(_, LocalFeatureKey::Attribute(name)) => {
                    let mut keys = routing_keys_for_input(&input);
                    for other in self.retained.facts.attribute_name_keys(name) {
                        if other != name {
                            keys.push(RoutingKey::AttributeName(other));
                        }
                    }
                    keys
                }
                InputKey::LocalFeature(..) | InputKey::State(..) => routing_keys_for_input(&input),
                _ => return true,
            };
            keys.into_iter().any(|key| {
                // Geometry independence depends on the routing key's rules, not the node.
                // Reuse that proof when several journal inputs reach the same key.
                if !checked_keys.insert(key) {
                    return false;
                }
                self.retained.routing.routes_for(key).iter().copied().any(|route| {
                    if !self
                        .retained
                        .routing
                        .route_is_live(route, &self.retained.program, &self.retained.programs)
                    {
                        return false;
                    }
                    let rule = self.retained.routing.rule_of(route);
                    !self.retained.program.declarations_are_complete_for(rule)
                        || self
                            .retained
                            .program
                            .declared_properties_of(rule)
                            .iter()
                            .any(|declared| {
                                crate::css::property_metadata::property_may_affect_layout_geometry(declared.property)
                            })
                })
            })
        })
    }

    /// Preserve the pending paint-only selector facts as the style change event established by a
    /// geometry read. Repeated reads advance the same boundary to the latest observed facts.
    /// Returning false means exact journalling coarsened while combining the facts, so the caller
    /// must settle style instead of reusing layout.
    pub fn defer_pending_transaction_for_geometry_read(&mut self, counters: &mut Counters) -> bool {
        debug_assert!(!self.host.flushing_deferred_geometry_journal);
        debug_assert!(self.host.tree_staging.is_empty());
        debug_assert!(!self.host.program_staging.is_dirty());
        debug_assert!(self.host.sheet_rule_replacement.is_none());
        debug_assert!(self.host.deferred_element_style_inputs.is_empty());
        debug_assert!(!self.host.initial_tree_bulk_load_is_pending);
        debug_assert!(self.host.journal.markers().is_empty());
        debug_assert!(
            self.host
                .journal
                .inputs()
                .all(|input| matches!(input.key, InputKey::LocalFeature(..) | InputKey::State(..)))
        );

        if self.host.journal.is_empty() {
            return true;
        }
        if self.host.deferred_geometry_journal.is_empty() {
            std::mem::swap(&mut self.host.journal, &mut self.host.deferred_geometry_journal);
        } else {
            self.host.deferred_geometry_journal.absorb_newer(
                &mut self.host.journal,
                &mut self.retained.memory,
                counters,
            );
        }
        self.host.deferred_geometry_journal.markers().is_empty()
    }

    /// Make the style transaction sealed by a geometry read current while preserving local facts
    /// recorded after it for the following style change event.
    pub fn begin_deferred_geometry_transaction_flush(&mut self) -> bool {
        debug_assert!(!self.host.flushing_deferred_geometry_journal);
        if self.host.deferred_geometry_journal.is_empty()
            || !self.host.journal.markers().is_empty()
            || !self.host.tree_staging.is_empty()
            || self.host.program_staging.is_dirty()
            || self.host.sheet_rule_replacement.is_some()
            || !self.host.deferred_element_style_inputs.is_empty()
            || self.host.initial_tree_bulk_load_is_pending
            || !self
                .host
                .journal
                .inputs()
                .all(|input| matches!(input.key, InputKey::LocalFeature(..) | InputKey::State(..)))
        {
            return false;
        }

        let later_inputs: Vec<NormalizedInput> = self.host.journal.inputs().collect();
        for input in &later_inputs {
            self.apply_to_facts_without_settling(input.key, input.old);
        }
        std::mem::swap(&mut self.host.journal, &mut self.host.deferred_geometry_journal);
        self.host.flushing_deferred_geometry_journal = true;
        true
    }

    /// Restore the local facts recorded after the geometry boundary once its transaction has been
    /// consumed.
    pub fn end_deferred_geometry_transaction_flush(&mut self) {
        assert!(self.host.flushing_deferred_geometry_journal);
        assert!(self.host.journal.is_empty());
        assert!(self.host.tree_staging.is_empty());
        assert!(!self.host.program_staging.is_dirty());
        assert!(self.host.sheet_rule_replacement.is_none());
        assert!(self.host.deferred_element_style_inputs.is_empty());

        std::mem::swap(&mut self.host.journal, &mut self.host.deferred_geometry_journal);
        let later_inputs: Vec<NormalizedInput> = self.host.journal.inputs().collect();
        for input in later_inputs {
            self.apply_to_facts_without_settling(input.key, input.new);
        }
        self.host.flushing_deferred_geometry_journal = false;
    }

    pub(super) fn merge_deferred_geometry_transaction(&mut self, counters: &mut Counters) {
        if self.host.flushing_deferred_geometry_journal || self.host.deferred_geometry_journal.is_empty() {
            return;
        }
        if self.host.journal.is_empty() {
            std::mem::swap(&mut self.host.journal, &mut self.host.deferred_geometry_journal);
            return;
        }
        self.host
            .deferred_geometry_journal
            .absorb_newer(&mut self.host.journal, &mut self.retained.memory, counters);
        std::mem::swap(&mut self.host.journal, &mut self.host.deferred_geometry_journal);
    }

    pub(crate) fn settle_batched_inputs(&mut self, counters: &mut Counters) {
        self.install_pending_matching_context();
        if !self.host.journal.contains_only_element_style_inputs() {
            self.discard_prepared_batch_matching_traversal();
        }
        self.discard_published_match_answers(counters);
    }

    #[must_use]
    pub(crate) fn node_arrival_is_pending(&self, node: StyleNodeID) -> bool {
        self.host.journal.pending_old(InputKey::TreeRelations(node)) == Some(InputValue::TreeRelations(None))
    }

    /// Returns whether the change joined the current transaction.
    pub(super) fn record(&mut self, key: InputKey, old: InputValue, new: InputValue, counters: &mut Counters) -> bool {
        self.discard_prepared_batch_matching_traversal();
        if let Some(node) = self.node_whose_arrival_carries(key) {
            // Every fact of an arriving element folds onto one key, so the journal holds one entry
            // per element rather than one per fact. Routing reads the facts back off the element.
            counters.bump(Counter::ArrivingNodeFactsFolded);
            self.host.journal.record(
                InputKey::LocalFeature(node, LocalFeatureKey::ArrivingFacts),
                InputValue::Feature(FeatureValue::Absent),
                InputValue::Feature(FeatureValue::Present),
                &mut self.retained.memory,
                counters,
            );
            return true;
        }
        self.host
            .journal
            .record(key, old, new, &mut self.retained.memory, counters);
        true
    }

    /// Whether a fact about an element is already carried by that element arriving.
    ///
    /// An element that connects in this transaction has its whole subtree put in the plan by its own
    /// tree delta, and the elements around it - the anchors of a relational query, the neighbours a
    /// sibling selector constrains, the parent whose emptiness moved - are reached from that delta
    /// too, not from anything the arriving element publishes about itself. So each of the facts it
    /// announces on the way in says nothing the arrival does not already say.
    ///
    /// What journalling them costs is the transaction's scratch budget: a page of a few thousand
    /// elements announces tens of thousands of them, and a journal that has to coarsen no longer
    /// knows which keys moved - which costs a restyle of the whole document, for elements whose
    /// arrival was already accounted for.
    ///
    /// The facts themselves are still applied: what is skipped is the journal entry, not the state.
    #[must_use]
    pub(super) fn node_whose_arrival_carries(&self, key: InputKey) -> Option<StyleNodeID> {
        let node = match key {
            InputKey::LocalFeature(node, feature) if feature != LocalFeatureKey::ArrivingFacts => node,
            InputKey::State(node, _) => node,
            _ => return None,
        };
        (self.host.journal.pending_old(InputKey::TreeRelations(node)) == Some(InputValue::TreeRelations(None)))
            .then_some(node)
    }

    #[inline]
    pub(super) fn stage_tree_row(
        &mut self,
        node: StyleNodeID,
        old_if_unstaged: Option<TreeRelations>,
        new: Option<TreeRelations>,
        counters: &mut Counters,
    ) {
        let old = self.host.tree_staging.current_row(node, old_if_unstaged);
        if old == new {
            return;
        }
        self.record(
            InputKey::TreeRelations(node),
            InputValue::TreeRelations(old),
            InputValue::TreeRelations(new),
            counters,
        );
        if old.is_some() && new.is_none() {
            counters.bump(Counter::TreeDepartureDeltas);
        }
        self.host.tree_staging.stage_row(node, old_if_unstaged, new);
    }

    pub(super) fn stage_connected_tree_row(
        &mut self,
        node: StyleNodeID,
        update: impl FnOnce(&mut TreeRelations),
        counters: &mut Counters,
    ) {
        let old = self
            .host
            .tree_staging
            .current_row(node, Some(self.settled_tree_relations(node)));
        let mut new = old.expect("a pending neighbour must remain connected");
        // A subtree can preallocate sibling identities before publishing their insertions. Such
        // a neighbour has no parent yet; its own insertion will supply its links. Synthesizing a
        // connected before-row here would hide that arrival from transaction normalization.
        if new.parent.is_none() {
            return;
        }
        update(&mut new);
        self.stage_tree_row(node, old, Some(new), counters);
    }

    pub(super) fn stage_first_child(&mut self, parent: StyleNodeID, child: Option<StyleNodeID>) {
        self.host
            .tree_staging
            .stage_first_child(parent, self.retained.tree.first_element_child(parent), child);
    }

    pub(super) fn settle_tree_staging_memory(&mut self) {
        let bytes = self.host.tree_staging.capacity_bytes();
        self.host
            .tree_staging_memory
            .resize_required_to(&mut self.retained.memory, bytes);
    }

    /// Install final staged relation rows at the transaction barrier.
    pub(super) fn apply_staged_tree_deltas(&mut self, counters: &mut Counters) {
        if self.host.tree_staging.is_empty() || self.host.tree_staging.is_applied() {
            return;
        }
        let staged_rows = self.host.tree_staging.dirty_rows();
        // Depth changes only for arrivals and for nodes whose parent differs from the resident one,
        // read before installation: the frozen before-side parent misses a move that a mid-transaction
        // application already installed, and a sibling-only row must not count as a moved parent.
        // A moved parent's subtree walk covers its moved descendants, so those are skipped below.
        let depth_recompute_nodes = self.depth_recompute_nodes(&staged_rows);

        for &(node, _, relations) in &staged_rows {
            let Some(relations) = relations else {
                self.retained.tree.set_parent_without_updating_depth(node, None);
                self.retained.tree.set_next_element_sibling(node, None);
                self.retained.tree.set_previous_element_sibling(node, None);
                self.retained
                    .tree
                    .set_assigned_slot(node, None, &mut self.retained.memory);
                continue;
            };
            self.retained
                .tree
                .set_parent_without_updating_depth(node, relations.parent);
            self.retained
                .tree
                .set_next_element_sibling(node, relations.next_element_sibling);
            self.retained
                .tree
                .set_previous_element_sibling(node, relations.previous_element_sibling);
            if relations.tree_scope != TreeScopeID::DOCUMENT {
                self.retained.tree.enable_tree_scopes(&mut self.retained.memory);
            }
            if self.retained.tree.has_tree_scopes() {
                self.retained.tree.set_tree_scope(node, relations.tree_scope);
            }
            self.retained
                .tree
                .set_assigned_slot(node, relations.assigned_slot, &mut self.retained.memory);
        }
        for (parent, _, child) in self.host.tree_staging.dirty_first_children() {
            self.retained.tree.set_first_element_child(parent, child);
        }
        for &(node, _, _) in &staged_rows {
            if !depth_recompute_nodes.contains(&node) {
                continue;
            }
            let parent_is_recomputed = self
                .tree
                .parent(node)
                .is_some_and(|parent| depth_recompute_nodes.contains(&parent));
            if !parent_is_recomputed {
                self.retained.tree.recompute_subtree_depth(node);
            }
        }
        let live_animation_overlays_before = self.retained.computed_group_sets.live_animation_overlay_records();
        let mut retired_nodes: Vec<StyleNodeID> = Vec::new();
        for &(node, _, relations) in &staged_rows {
            if relations.is_some() || !self.retained.tree.is_live(node) {
                continue;
            }
            self.retained.winner_groups.remove(node);
            self.retained.computed_group_sets.remove(node);
            self.retained.drop_demand_pseudo_records(node);
            self.retained.nodes_with_substituted_records.remove(&node);
            self.retained.nodes_with_tree_counting_records.remove(&node);
            self.retained.nodes_owing_a_transition_registration.remove(&node);
            self.retained.nodes_owing_explicit_inheritance.remove(&node);
            self.retained.children_explicitly_inherit_marks.remove(&node);
            self.retained.engine_row_child_facts.remove(&node);
            self.retained.nodes_owing_an_animation_sample.remove(&node);
            self.retained.pseudo_settles_owed.remove(&node);
            self.retained.rows_sampled_in_pass.remove(&node);
            self.retained
                .pseudo_elements_sampled_in_pass
                .retain(|(owner, _), _| *owner != node);
            self.retained
                .pseudo_element_transition_steps_decided_in_pass
                .retain(|(owner, _), _| *owner != node);
            self.retained.release_transition_baselines_of(node);
            self.retained
                .nodes_owing_animation_definitions
                .retain(|(owner, _), _| *owner != node);
            retired_nodes.push(node);
        }
        if !retired_nodes.is_empty() {
            self.retained.layout_style_snapshots.retire(&retired_nodes);
            for &node in &retired_nodes {
                self.retained.container_query_inputs.clear(node);
                self.host.held_style_records.remove(&node);
                // An identity can be minted again for another element, so a retained environment
                // must not outlive the element that installed it.
                if let Some(held) = self.retained.element_custom_property_data.remove(&node) {
                    self.host.retired_custom_property_data.extend(held.data);
                }
                self.retained.sampled_custom_property_environments.remove(&node);
                self.retained
                    .sampled_pseudo_element_custom_property_environments
                    .retain(|(owner, _), _| *owner != node);
                self.retained.environment_move_recompute_nodes.remove(&node);
                self.retained.size_container_queries.retire(node);
                self.retained.anchor_names.retire(node);
            }
            if !self.retained.pseudo_element_custom_property_data.is_empty() {
                let retired: HashSet<StyleNodeID> = retired_nodes.iter().copied().collect();
                let keys: Vec<_> = self
                    .retained
                    .pseudo_element_custom_property_data
                    .keys()
                    .filter(|(node, _)| retired.contains(node))
                    .copied()
                    .collect();
                for key in keys {
                    if let Some(held) = self.retained.pseudo_element_custom_property_data.remove(&key) {
                        self.host.retired_custom_property_data.extend(held.data);
                    }
                }
            }
            self.retained
                .tree
                .retire_elements(&retired_nodes, &mut self.retained.memory);
            // An identity can be minted again for another element, so the top layer cannot be left
            // naming one that has been given up.
            if !self.retained.top_layer_elements.is_empty() {
                self.retained
                    .top_layer_elements
                    .retain(|member| !retired_nodes.contains(member));
            }
            self.retained.css_defined_animations.retire(&retired_nodes);
            self.retained.animation_timing_rows.retire(&retired_nodes);
            self.retained.element_transitions.retire(&retired_nodes);
            self.retained.animation_effect_descriptions.retire(&retired_nodes);
            self.retained
                .deferred_pseudo_element_observable_nodes
                .retain(|member| !retired_nodes.contains(member));
            self.host
                .latent_deferred_pseudo_element_style_inputs
                .retain(|input| input.key.style_node().is_none_or(|node| !retired_nodes.contains(&node)));
            self.settle_deferred_element_style_input_memory();
            let live_animation_overlays_after = self.retained.computed_group_sets.live_animation_overlay_records();
            self.settle_computed_memory();
            counters.add(
                Counter::AnimationOverlaySlotsReleased,
                (live_animation_overlays_before - live_animation_overlays_after) as u64,
            );
            counters.set(
                Counter::LiveAnimationOverlayRecords,
                live_animation_overlays_after as u64,
            );
            counters.add(Counter::StyleNodesRetired, retired_nodes.len() as u64);
        }
        self.host.tree_staging.mark_applied();
        self.publish_budget_inputs();
    }

    /// Name the node a style scope belongs to. A shadow root is a scope and a subtree at once, which
    /// is what lets a sheet attached there be bounded by the tree it decides in.
    /// Record that a tree scope decides with the document's author sheets as well as its own.
    pub fn set_tree_scope_uses_document_sheets(&mut self, tree_scope: TreeScopeID) {
        let previous = self
            .host
            .program_staging
            .scopes_using_document_sheets
            .current(tree_scope, || {
                self.retained.program.scope_uses_document_sheets(tree_scope)
            });
        if previous {
            return;
        }
        self.host.program_staging.scopes_using_document_sheets.stage(
            tree_scope,
            || self.retained.program.scope_uses_document_sheets(tree_scope),
            true,
        );
        self.host
            .program_staging
            .base_version
            .get_or_insert(self.retained.program.version());
        self.invalidate_scope_program(tree_scope);
    }
}

#[cfg(test)]
mod random_base_value_tests {
    use super::*;

    #[test]
    fn random_base_value_scopes_are_retained_by_the_style_engine() {
        let mut engine = StyleEngineState::new_for_replay(DeviceClass::ForegroundDesktop);
        let first = StyleNodeID::from_raw(1).unwrap();
        let second = StyleNodeID::from_raw(2).unwrap();
        let name = "--shared".encode_utf16().collect::<Vec<_>>();

        let document_value = engine.ensure_random_base_value(first, &name, true);
        assert_eq!(document_value, engine.ensure_random_base_value(second, &name, true));
        assert_eq!(engine.retained.random_base_values.len(), 1);

        let element_value = engine.ensure_random_base_value(first, &name, false);
        assert_eq!(element_value, engine.ensure_random_base_value(first, &name, false));
        engine.ensure_random_base_value(second, &name, false);
        assert_eq!(engine.retained.random_base_values.len(), 3);
    }
}

impl StyleEngineState {
    /// Add a style rule that only applies inside a scope, naming the scope's root selectors.
    ///
    /// `before` places the rule immediately ahead of an existing one instead of at the end. A rule
    /// arriving in the middle of a sheet takes an order token between its neighbours: nothing else
    /// is renumbered, and no other rule's identity or compiled program is touched.
    pub fn add_style_rule_in_scope(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        selectors: &[&CompiledSelector],
        namespaces: NamespaceScope,
        scope: &ScopeChain<'_>,
        counters: &mut Counters,
    ) -> RuleID {
        let rule = match before {
            Some(before) => self.insert_rule_before(before, RuleKind::Style, counters),
            None => self
                .reuse_replaced_style_rule(sheet, counters)
                .unwrap_or_else(|| self.append_rule(sheet, None, RuleKind::Style, counters)),
        };
        let previous_program = self
            .replacement_rule(rule)
            .and_then(|replacement| replacement.version.selector_program);
        let program = self.compile_selectors(selectors, namespaces, scope, previous_program, counters);
        if previous_program != Some(program) {
            self.add_routing_rule(rule, program);
        }

        let mut version = self.current_rule_version(rule);
        version.selector_program = Some(program);
        self.replace_rule_version(rule, version, counters);
        counters.bump(Counter::StyleRulesCompiled);
        rule
    }

    pub(crate) fn selector_program_for_rule(&self, rule: RuleID) -> &SelectorProgram {
        let program = self
            .current_rule_version(rule)
            .selector_program
            .expect("a style rule must have a selector program");
        self.retained.programs.get(program)
    }

    /// Record that a sheet declared or gave up a cascade layer.
    ///
    /// The declaration contributes no declarations and matches nothing: what it does is fix the order
    /// of the layers every rule referencing them sits in. A layer name belongs to the tree scope the
    /// sheet is attached to, so what moves is that scope's layer order - one input per scope the sheet
    /// decides in.
    pub fn record_layer_statement(&mut self, sheet: SheetID, counters: &mut Counters) {
        for scope in self.retained.program.sheet_scopes(sheet) {
            self.record_layer_topology_change(scope, counters);
        }
    }

    pub(super) fn add_named_rule(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        kind: RuleKind,
        name: StyleAtomID,
        counters: &mut Counters,
    ) -> RuleID {
        let rule = match before {
            Some(before) => self.insert_rule_before(before, kind, counters),
            None => self.append_rule(sheet, None, kind, counters),
        };
        let mut version = self.current_rule_version(rule);
        version.declared_name = Some(name);
        self.replace_rule_version(rule, version, counters);
        rule
    }

    /// Give an existing rule a new selector list, keeping its identity and its position.
    ///
    /// Editing `selectorText` is one rule changing, not the sheet being rebuilt. The rule keeps its
    /// order token, so nothing around it is renumbered, and the journal sees exactly one selector
    /// field move.
    pub fn replace_style_rule_selectors(
        &mut self,
        rule: RuleID,
        selectors: &[&CompiledSelector],
        namespaces: NamespaceScope,
        scope: &ScopeChain<'_>,
        counters: &mut Counters,
    ) {
        let program = self.compile_selectors(selectors, namespaces, scope, None, counters);
        self.add_routing_rule(rule, program);

        let mut version = self.current_rule_version(rule);
        version.selector_program = Some(program);
        self.replace_rule_version(rule, version, counters);
        self.settle_program();
        counters.bump(Counter::StyleRulesCompiled);
    }

    /// Report that a rule's declarations moved, without touching anything else about it.
    ///
    /// The block's contents changed even where the CSSOM object did not, so what makes this a
    /// change is a version rather than the object's address. The rule keeps its identity, its
    /// position, and its selector program, so the journal sees one field move and routing reaches
    /// exactly the elements that rule matches.
    pub fn record_rule_declarations_changed(&mut self, rule: RuleID, block_version: u32, counters: &mut Counters) {
        let mut version = self.current_rule_version(rule);
        version.declaration_block = Some(DeclarationBlockID(block_version));
        self.replace_rule_version(rule, version, counters);
        self.settle_program();
    }

    /// Grant and mint `out.len()` text identities in one call, for an engine with no host.
    #[cfg(test)]
    pub fn allocate_text_style_nodes(&mut self, out: &mut [u32], counters: &mut Counters) {
        self.grant_text_style_nodes(out);
        let nodes: Vec<StyleNodeID> = out.iter().map(|&raw| StyleNodeID::from_raw(raw).unwrap()).collect();
        self.mint_text_style_nodes(&nodes, counters);
    }

    /// Grant and mint `out.len()` element identities in one call, for an engine with no host.
    #[cfg(test)]
    pub fn allocate_style_nodes(&mut self, out: &mut [u32], counters: &mut Counters) {
        self.grant_style_nodes(out);
        let nodes: Vec<StyleNodeID> = out.iter().map(|&raw| StyleNodeID::from_raw(raw).unwrap()).collect();
        self.mint_style_nodes(&nodes, counters);
    }

    /// Hand the host `out.len()` text identities to mint on its own. See [`Self::grant_style_nodes`].
    pub fn grant_text_style_nodes(&mut self, out: &mut [u32]) {
        for slot in out.iter_mut() {
            *slot = self.retained.tree.grant_text(&mut self.retained.memory).raw();
        }
    }

    /// Hand the host `out.len()` element identities to mint on its own.
    ///
    /// The host names a node the moment it connects and writes to the name at once, while the
    /// engine may be in the middle of a pass. So it mints from identities granted to it ahead of
    /// time, and the mint crosses with the input transaction, in order with what it wrote since.
    pub fn grant_style_nodes(&mut self, out: &mut [u32]) {
        for slot in out.iter_mut() {
            *slot = self.retained.tree.grant_element(&mut self.retained.memory).raw();
        }
    }

    /// Bring text identities the host minted into the tree.
    pub fn mint_text_style_nodes(&mut self, nodes: &[StyleNodeID], counters: &mut Counters) {
        for &node in nodes {
            self.retained.tree.mint_text(node, &mut self.retained.memory);
            counters.bump(Counter::StyleNodesAllocated);
        }
    }

    /// Bring element identities the host minted into the tree.
    ///
    /// A freshly minted element holds no custom-property environment yet: installing its style
    /// gives it one. An identity minted again holds nothing of the element it named before.
    pub fn mint_style_nodes(&mut self, nodes: &[StyleNodeID], counters: &mut Counters) {
        for &node in nodes {
            self.retained.tree.mint_element(node, &mut self.retained.memory);
            self.retained.element_custom_property_data.remove(&node);
            counters.bump(Counter::StyleNodesAllocated);
        }
        self.publish_budget_inputs();
    }

    pub fn record_input(&mut self, key: InputKey, old: InputValue, new: InputValue, counters: &mut Counters) {
        if self.record(key, old, new, counters) {
            self.apply_to_facts_without_settling(key, new);
        }
    }

    /// Record an exact style reaction for every flat-tree descendant of a node.
    ///
    /// The reaction is one C++ derived from a reaction it applied to `root`, not a fact only C++
    /// holds: a descendant is here because what it inherits moved, which the engine can settle
    /// for itself wherever the record computation admits it. Recording these as inputs the C++
    /// computation owes would decline the engine's own record for every one of them.
    pub fn record_flat_tree_descendant_style_inputs(
        &mut self,
        root: StyleNodeID,
        reaction: u8,
        inherited_style_groups: u8,
    ) {
        if reaction == 0 {
            return;
        }
        let mut descendants = Vec::new();
        self.for_each_flat_tree_descendant(root, |node| descendants.push(node));
        for node in descendants {
            if reaction
                == transaction::STYLE_REACTION_RECOMPUTE_STYLE
                    | transaction::STYLE_REACTION_PSEUDO_INPUTS_MAY_HAVE_CHANGED
            {
                self.record_deferred_pseudo_element_style_input(node);
            } else {
                self.record_derived_element_style_input(node, reaction, inherited_style_groups);
            }
        }
    }

    /// Record one member of a flat FFI batch without repeatedly settling the fact-store capacity.
    ///
    /// The bridge has already applied every tree delta before it publishes facts. It can therefore
    /// identify facts carried by a node's arrival once per node instead of probing and replacing
    /// the same journal key once per fact.
    pub(crate) fn record_batched_input(
        &mut self,
        key: InputKey,
        old: InputValue,
        new: InputValue,
        arriving_node: bool,
        counters: &mut Counters,
    ) {
        if arriving_node {
            debug_assert!(matches!(key, InputKey::LocalFeature(..) | InputKey::State(..)));
            counters.bump(Counter::ArrivingNodeFactsFolded);
            counters.bump(Counter::RawMutationRecords);
            counters.bump(Counter::LocalFeatureDeltas);
            self.apply_to_facts_without_settling(key, new);
        } else if self.record(key, old, new, counters) {
            self.apply_to_facts_without_settling(key, new);
        }
    }

    /// Stage one structural delta and the neighbour rows it derives.
    pub(super) fn stage_tree_delta(
        &mut self,
        node: StyleNodeID,
        old: Option<TreeRelations>,
        new: Option<TreeRelations>,
        counters: &mut Counters,
    ) {
        if let Some(old) = old {
            if let Some(previous) = old.previous_element_sibling {
                self.stage_connected_tree_row(
                    previous,
                    |relations| {
                        relations.next_element_sibling = old.next_element_sibling;
                    },
                    counters,
                );
            } else if let Some(parent) = old.parent {
                self.stage_first_child(parent, old.next_element_sibling);
            }
            if let Some(next) = old.next_element_sibling {
                self.stage_connected_tree_row(
                    next,
                    |relations| {
                        relations.previous_element_sibling = old.previous_element_sibling;
                    },
                    counters,
                );
            }
        }
        if let Some(new) = new {
            if let Some(previous) = new.previous_element_sibling {
                self.stage_connected_tree_row(
                    previous,
                    |relations| {
                        relations.next_element_sibling = Some(node);
                    },
                    counters,
                );
            } else if let Some(parent) = new.parent {
                self.stage_first_child(parent, Some(node));
            }
            if let Some(next) = new.next_element_sibling {
                self.stage_connected_tree_row(
                    next,
                    |relations| {
                        relations.previous_element_sibling = Some(node);
                    },
                    counters,
                );
            }
        }
        self.stage_tree_row(node, old, new, counters);
        self.settle_tree_staging_memory();
    }

    /// Attach a compiled program at the end of a scope's sheet order.
    pub(super) fn attach_sheet(&mut self, sheet: SheetID, tree_scope: TreeScopeID, counters: &mut Counters) {
        self.restore_routing_for_reattached_sheet(sheet);
        let mut sheets = self.current_sheets_in_scope(tree_scope).to_vec();
        let previous_position = sheets.iter().position(|&candidate| candidate == sheet);
        let was_attached = previous_position.is_some();
        sheets.retain(|&candidate| candidate != sheet);
        sheets.push(sheet);
        let order_changed = previous_position.is_some_and(|previous| previous != sheets.len() - 1);
        self.stage_sheets_in_scope(tree_scope, sheets);
        self.record_attachment(sheet, tree_scope, was_attached, true, counters);
        if order_changed {
            self.record_sheet_order_change(tree_scope, counters);
        }
    }

    /// Attach at a position established before the sheet finished loading. Network completion order
    /// does not determine cascade order.
    pub(super) fn attach_sheet_before(
        &mut self,
        sheet: SheetID,
        before: SheetID,
        tree_scope: TreeScopeID,
        counters: &mut Counters,
    ) {
        self.restore_routing_for_reattached_sheet(sheet);
        let mut sheets = self.current_sheets_in_scope(tree_scope).to_vec();
        let previous_position = sheets.iter().position(|&candidate| candidate == sheet);
        let was_attached = previous_position.is_some();
        sheets.retain(|&candidate| candidate != sheet);
        let position = sheets
            .iter()
            .position(|&candidate| candidate == before)
            .unwrap_or(sheets.len());
        sheets.insert(position, sheet);
        let order_changed = previous_position.is_some_and(|previous| previous != position);
        self.stage_sheets_in_scope(tree_scope, sheets);
        // A sheet arriving in the middle is not the scope's order changing. Order is kept as tokens
        // precisely so an insertion writes one label and renumbers nothing, so every sheet already
        // attached keeps the priority it had against every other. What is new is one more competitor
        // for the declarations it makes, and the elements that competition can reach are exactly the
        // ones its own rules match, which the attachment recorded above already names. A sheet that
        // was attached a moment ago and is arriving again can be a move, whose order delta is
        // recorded below.
        self.record_attachment(sheet, tree_scope, was_attached, true, counters);
        if order_changed {
            self.record_sheet_order_change(tree_scope, counters);
        }
    }

    /// Attach a sheet immediately before another sheet in the same scope, or at the end when that
    /// sheet is not attached there. Order tokens stay inside the engine: callers name neighbours,
    /// never positions.
    pub fn attach_sheet_before_sheet(
        &mut self,
        sheet: SheetID,
        before: Option<SheetID>,
        tree_scope: TreeScopeID,
        counters: &mut Counters,
    ) {
        let before = before.filter(|&before| self.current_sheets_in_scope(tree_scope).contains(&before));
        match before {
            Some(before) => self.attach_sheet_before(sheet, before, tree_scope, counters),
            None => self.attach_sheet(sheet, tree_scope, counters),
        }
    }

    pub fn detach_sheet(&mut self, sheet: SheetID, tree_scope: TreeScopeID, counters: &mut Counters) {
        let sheets = self.current_sheets_in_scope(tree_scope);
        let Some(position) = sheets.iter().position(|&candidate| candidate == sheet) else {
            return;
        };
        let mut sheets = sheets.to_vec();
        sheets.remove(position);
        self.stage_sheets_in_scope(tree_scope, sheets);
        self.record_attachment(sheet, tree_scope, true, false, counters);
        self.retained.routing_needs_detachment_sweep = true;
    }
}

impl StyleEngineState {
    /// Add a `@keyframes` rule, which matches no element and is found by the name it declares.
    ///
    /// It has to be in the program at all for a change to it to be an input, and it has to carry its
    /// name for that input to reach the animations referencing it.
    pub fn add_keyframes_rule(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        name: StyleAtomID,
        counters: &mut Counters,
    ) -> RuleID {
        self.add_named_rule(sheet, before, RuleKind::Keyframes, name, counters)
    }

    /// Add an `@property` rule, which registers the custom property it names. Registering one changes
    /// how every element that declares or references it computes, which the custom-property index
    /// knows and selector matching cannot say.
    pub fn add_property_rule(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        name: StyleAtomID,
        counters: &mut Counters,
    ) -> RuleID {
        self.add_named_rule(sheet, before, RuleKind::Property, name, counters)
    }

    /// Add a rule that matches no element and is not found by name either, so that a change to it is
    /// an input at all. What it reaches is decided by its kind.
    pub fn add_non_matching_rule(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        kind: RuleKind,
        counters: &mut Counters,
    ) -> RuleID {
        match before {
            Some(before) => self.insert_rule_before(before, kind, counters),
            None => self.append_rule(sheet, None, kind, counters),
        }
    }

    /// Stage a structural change. The normalized transaction installs the final relation rows at
    /// the next observation boundary.
    pub fn record_tree_delta(
        &mut self,
        node: StyleNodeID,
        old: Option<TreeRelations>,
        new: Option<TreeRelations>,
        counters: &mut Counters,
    ) {
        if old != new {
            self.stage_tree_delta(node, old, new, counters);
        }
    }

    /// Record a registration made through `CSS.registerProperty()`. Stylesheet registrations are
    /// already represented by their `@property` rule's program input.
    pub fn record_custom_property_registration_change(&mut self, name: StyleAtomID, counters: &mut Counters) {
        self.record_input(
            InputKey::CustomPropertyRegistration(name),
            InputValue::Flag(false),
            InputValue::Flag(true),
            counters,
        );
    }

    /// Record a state publication against the settled value StyleEngine already owns.
    ///
    /// State invalidators may conservatively republish a related group of pseudo-classes when one
    /// member changes. The settled fact is therefore the old side; assuming every publication is
    /// a Boolean toggle would invent transitions for the unchanged members of that group.
    pub(crate) fn record_batched_state(
        &mut self,
        node: StyleNodeID,
        fact: StateFact,
        new_value: bool,
        arriving_node: bool,
        counters: &mut Counters,
    ) {
        let old_value = self.retained.facts.states_of_node(node).contains(fact);
        self.record_batched_input(
            InputKey::State(node, fact),
            InputValue::State(old_value),
            InputValue::State(new_value),
            arriving_node,
            counters,
        );
    }

    /// Install the fixed facts carried by one element arrival. The tree row already routes the
    /// arriving element, so facts which can change later update their columns without adding one
    /// journal entry apiece.
    pub(crate) fn record_element_arrival(
        &mut self,
        node: StyleNodeID,
        arrival: &super::bridge::FfiElementArrival,
        custom_states: &[StyleAtomID],
        arriving_node: bool,
        counters: &mut Counters,
    ) {
        debug_assert!(arriving_node);
        self.retained
            .facts
            .set_namespace(node, StyleAtomID(arrival.namespace_atom));
        self.retained.facts.set_is_slot(node, arrival.is_slot);
        let mut publish_feature = |feature, value| {
            self.record_batched_input(
                InputKey::LocalFeature(node, feature),
                InputValue::Feature(FeatureValue::Absent),
                InputValue::Feature(value),
                arriving_node,
                counters,
            );
        };
        if arrival.language_atom != 0 {
            publish_feature(
                LocalFeatureKey::Language,
                FeatureValue::Atom(StyleAtomID(arrival.language_atom)),
            );
        }
        if arrival.directionality_atom != 0 {
            publish_feature(
                LocalFeatureKey::Directionality,
                FeatureValue::Atom(StyleAtomID(arrival.directionality_atom)),
            );
        }
        if arrival.heading_level != 0 {
            publish_feature(
                LocalFeatureKey::HeadingLevel,
                FeatureValue::Number(u32::from(arrival.heading_level)),
            );
        }
        self.retained
            .computed_group_sets
            .set_adjustment_facts(node, arrival.adjustment_facts);
        self.retained
            .computed_group_sets
            .set_associated_pseudo_kind(node, arrival.associated_pseudo_kind_plus_one);
        self.retained
            .computed_group_sets
            .set_construction_facts(node, arrival.construction_facts, arrival.box_kind);
        for &state in custom_states {
            self.record_batched_input(
                InputKey::LocalFeature(node, LocalFeatureKey::CustomState(state)),
                InputValue::Feature(FeatureValue::Absent),
                InputValue::Feature(FeatureValue::Present),
                arriving_node,
                counters,
            );
        }
        self.retained
            .facts
            .set_custom_states(node, custom_states, &mut self.retained.memory);
    }

    /// Record the shadow parts an element exposes.
    ///
    /// A part is a fact about the element like a class is: posted so `::part()` rules can be
    /// enumerated from it, and journalled so a change to the `part` attribute routes. The plain
    /// name set is derived here from the name-to-host pairs exact matching needs, so the two views
    /// cannot disagree.
    pub fn set_element_parts(
        &mut self,
        node: StyleNodeID,
        pairs: &[(StyleAtomID, StyleNodeID)],
        counters: &mut Counters,
    ) {
        let mut parts = Vec::new();
        for &(part, _) in pairs {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        if self.retained.facts.parts_of(node) != parts {
            let previous: Vec<StyleAtomID> = self.retained.facts.parts_of(node).to_vec();
            for part in previous.iter().filter(|part| !parts.contains(part)) {
                self.record_input(
                    InputKey::LocalFeature(node, LocalFeatureKey::Part(*part)),
                    InputValue::Feature(FeatureValue::Present),
                    InputValue::Feature(FeatureValue::Absent),
                    counters,
                );
            }
            for part in parts.iter().filter(|part| !previous.contains(part)) {
                self.record_input(
                    InputKey::LocalFeature(node, LocalFeatureKey::Part(*part)),
                    InputValue::Feature(FeatureValue::Absent),
                    InputValue::Feature(FeatureValue::Present),
                    counters,
                );
            }
            self.retained.facts.set_parts(node, &parts, &mut self.retained.memory);
        }
        self.retained
            .tree
            .set_part_hosts(node, pairs, &mut self.retained.memory);
    }

    /// Report the outermost host a `::part()` rule can address this element from.
    ///
    /// What an `exportparts` change moves is which scopes can name an element, not which names it
    /// carries - the forwarded name is usually the one it already had. So the exposure is the fact,
    /// and an element whose reach did not move says nothing.
    pub fn set_element_part_exposure(&mut self, node: StyleNodeID, exposure: StyleAtomID, counters: &mut Counters) {
        let previous = self.retained.facts.part_exposure_of(node);
        if previous == exposure {
            return;
        }
        self.record_input(
            InputKey::LocalFeature(node, LocalFeatureKey::PartExposure),
            InputValue::Feature(RetainedState::atom_or_absent(previous)),
            InputValue::Feature(RetainedState::atom_or_absent(exposure)),
            counters,
        );
    }

    pub fn set_element_heading_level(&mut self, node: StyleNodeID, level: u8, counters: &mut Counters) {
        let previous = self.retained.facts.heading_level_of(node);
        if previous == level {
            return;
        }
        self.record_input(
            InputKey::LocalFeature(node, LocalFeatureKey::HeadingLevel),
            InputValue::Feature(FeatureValue::Number(u32::from(previous))),
            InputValue::Feature(FeatureValue::Number(u32::from(level))),
            counters,
        );
    }

    pub fn set_element_language(&mut self, node: StyleNodeID, language: StyleAtomID, counters: &mut Counters) {
        let previous = self.retained.facts.language_of(node);
        if previous == language {
            return;
        }
        self.record_input(
            InputKey::LocalFeature(node, LocalFeatureKey::Language),
            InputValue::Feature(RetainedState::atom_or_absent(previous)),
            InputValue::Feature(RetainedState::atom_or_absent(language)),
            counters,
        );
    }

    /// Report the element's resolved directionality, which `:dir()` tests.
    pub fn set_element_directionality(
        &mut self,
        node: StyleNodeID,
        directionality: StyleAtomID,
        counters: &mut Counters,
    ) {
        let previous = self.retained.facts.directionality_of(node);
        if previous == directionality {
            return;
        }
        self.record_input(
            InputKey::LocalFeature(node, LocalFeatureKey::Directionality),
            InputValue::Feature(RetainedState::atom_or_absent(previous)),
            InputValue::Feature(RetainedState::atom_or_absent(directionality)),
            counters,
        );
    }

    /// Replace the custom states an element is in.
    ///
    /// A custom state is a named fact about one element, exactly like a class, so it is published as
    /// one: the names that arrived and the names that left are each a local feature moving, and
    /// `:state()` reaches its subjects through the same postings every other name does.
    pub fn set_element_custom_states(&mut self, node: StyleNodeID, states: &[StyleAtomID], counters: &mut Counters) {
        if self.retained.facts.custom_states_of(node) == states {
            return;
        }
        let previous: Vec<StyleAtomID> = self.retained.facts.custom_states_of(node).to_vec();
        for state in previous.iter().filter(|state| !states.contains(state)) {
            self.record_input(
                InputKey::LocalFeature(node, LocalFeatureKey::CustomState(*state)),
                InputValue::Feature(FeatureValue::Present),
                InputValue::Feature(FeatureValue::Absent),
                counters,
            );
        }
        for state in states.iter().filter(|state| !previous.contains(state)) {
            self.record_input(
                InputKey::LocalFeature(node, LocalFeatureKey::CustomState(*state)),
                InputValue::Feature(FeatureValue::Absent),
                InputValue::Feature(FeatureValue::Present),
                counters,
            );
        }
        self.retained
            .facts
            .set_custom_states(node, states, &mut self.retained.memory);
    }
}

impl StyleEngineState {
    #[cfg(feature = "style-recording")]
    pub(crate) fn add_replayed_style_rule(
        &mut self,
        sheet: SheetID,
        before: Option<RuleID>,
        selector_program: SelectorProgram,
        counters: &mut Counters,
    ) -> RuleID {
        let rule = match before {
            Some(before) => self.insert_rule_before(before, RuleKind::Style, counters),
            None => self
                .reuse_replaced_style_rule(sheet, counters)
                .unwrap_or_else(|| self.append_rule(sheet, None, RuleKind::Style, counters)),
        };
        let previous_program = self
            .replacement_rule(rule)
            .and_then(|replacement| replacement.version.selector_program);
        let program = self.retained.programs.add(selector_program);
        self.retained.selector_programs_need_sweep |= previous_program.is_some();
        self.retained.programs.settle_memory(&mut self.retained.memory);
        if previous_program != Some(program) {
            self.add_routing_rule(rule, program);
        }
        let mut version = self.current_rule_version(rule);
        version.selector_program = Some(program);
        self.replace_rule_version(rule, version, counters);
        counters.bump(Counter::StyleRulesCompiled);
        rule
    }

    #[cfg(feature = "style-recording")]
    pub(crate) fn replace_replayed_style_rule_selectors(
        &mut self,
        rule: RuleID,
        selector_program: SelectorProgram,
        counters: &mut Counters,
    ) {
        let program = self.retained.programs.add(selector_program);
        self.retained.selector_programs_need_sweep = true;
        self.retained.programs.settle_memory(&mut self.retained.memory);
        self.add_routing_rule(rule, program);
        let mut version = self.current_rule_version(rule);
        version.selector_program = Some(program);
        self.replace_rule_version(rule, version, counters);
        self.settle_program();
        counters.bump(Counter::StyleRulesCompiled);
    }
}

// The font length-resolution context one row resolves its font properties against, answered from
// retained state alone. The host used to build this per row from live state: the navigable's
// viewport, the inheriting element's `ComputedValues` and its platform font's pixel metrics, and
// the root element's `ComputedValues`. Every one of those is already retained - the host's
// per-update document environment holds the viewport, the root metrics and the initial font, and
// a record's font group holds the five metrics `Length::FontMetrics` carries - so the row can be
// answered without reading the DOM.
