/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The custom-property environment of an element the engine computes a record for.
//!
//! The cascade decides a custom property the way it decides a longhand - the highest-priority
//! declaration of the name wins - but the engine keeps no winner column per name: the names are
//! unbounded, and nothing but the environment reads them. So an element's custom declarations
//! cascade here, by name, when its environment is computed: over the matched rules and the inline
//! style, into the store the C++ resolver builds from, resolved by the same Rust resolver against
//! the environment the element inherits.

use std::ffi::c_void;
use std::ops::ControlFlow;
use std::sync::Arc;

use super::*;
use crate::css::cascaded_properties::{
    CallbackFreeParseOutcome, FfiCascadeResolutionContext, FfiCustomPropertyDriveInput,
    custom_property_value_is_callback_free as custom_property_value_is_engine_resolvable,
    destroy_resolved_custom_properties, drive_custom_property_resolution, parse_substituted_source,
    parse_substituted_without_callbacks,
};
use crate::css::custom_properties::{
    CustomPropertyRegistry, CustomPropertyStore, NativeVarResolution, prepare_var_resolution_environment,
};
use crate::css::ffi_support::FfiUtf16View;
use crate::css::parser::value_parser::ParseOutcome;
use crate::css::style_value::{RetainedStyleValueData, StyleValueData, release_style_value};
use custom_property_environments::CascadedCustomProperty;

/// The resolution context the engine substitutes under: the stores alone, with no callback into
/// C++ - what the engine cannot resolve without one is left to C++ before this is built.
fn engine_resolution_context(
    parse_context: &crate::css::parser::value_parser::ParseContext,
    store: *const c_void,
    inheritance_store: *const c_void,
    registry: *const c_void,
) -> FfiCascadeResolutionContext {
    FfiCascadeResolutionContext {
        parse_context: std::ptr::from_ref(parse_context).cast(),
        media_environment: std::ptr::null(),
        load_media_environment: None,
        custom_property_store: store,
        animated_custom_property_store: std::ptr::null(),
        animated_custom_property_base_store: std::ptr::null(),
        inheritance_custom_property_store: inheritance_store,
        custom_property_registry: registry,
        root_custom_property_name: FfiUtf16View {
            ascii: std::ptr::null(),
            utf16: std::ptr::null(),
            length: 0,
        },
        attributes: std::ptr::null(),
        attribute_count: 0,
        attribute_names_are_ascii_case_insensitive: false,
        custom_functions: std::ptr::null(),
        custom_function_count: 0,
        custom_function_scope_identity: 0,
        custom_function_visibilities: std::ptr::null(),
        custom_function_visibility_count: 0,
        style_query_length_resolution_context: std::ptr::null(),
        style_query_dependencies: std::ptr::null_mut(),
        callback_context: std::ptr::null_mut(),
    }
}

/// Whether a token stream is a substitution the engine resolves itself: one whose only
/// substitution functions are `var()` references.
pub(super) fn value_is_engine_resolvable_substitution(value: &StyleValueData) -> bool {
    matches!(value, StyleValueData::Unresolved { .. }) && custom_property_value_is_engine_resolvable(value)
}

impl RetainedState {
    /// Hand each of a node's element-target matches, with the cascade inputs its priority is
    /// computed from, to `visit`, stopping when it breaks. `None` when the node has no answer to read.
    fn try_for_each_element_match(
        &self,
        node: StyleNodeID,
        visit: impl FnMut(RuleID, TreeScopeID, Specificity, u32) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        self.try_for_each_match(node, None, visit)
    }

    /// The element's matches when `pseudo` is `None`, else the matches for that pseudo-element.
    fn try_for_each_match(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        mut visit: impl FnMut(RuleID, TreeScopeID, Specificity, u32) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        let wanted =
            |target: Option<tree::PseudoElementTarget>| target.map(|target| target.kind.0) == pseudo.map(u16::from);
        if let Some((published, answer)) = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        ) && let Some(matches) = published.matches_for(answer)
        {
            for entry in matches.iter().filter(|entry| wanted(entry.pseudo_element)) {
                if visit(entry.rule, entry.tree_scope, entry.specificity, entry.scope_proximity).is_break() {
                    return Some(ControlFlow::Break(()));
                }
            }
            return Some(ControlFlow::Continue(()));
        }
        let Lookup::Known(answer) = self.retained_match_answer(node) else {
            return None;
        };
        for rule_match in answer.iter() {
            let entry = &self.programs.get(rule_match.program).entries()[rule_match.entry as usize];
            if !wanted(entry.pseudo_element) {
                continue;
            }
            if visit(
                rule_match.rule,
                rule_match.tree_scope,
                entry.specificity,
                rule_match.scope_proximity,
            )
            .is_break()
            {
                return Some(ControlFlow::Break(()));
            }
        }
        Some(ControlFlow::Continue(()))
    }

    /// Whether anything in the document declares a custom property. Nothing declaring one means
    /// every environment is the inherited one, and no cascade need look.
    fn any_custom_property_is_declared(&self) -> bool {
        self.program.any_rule_declares_custom_properties() || self.facts.any_element_declares_custom_properties()
    }

    pub(super) fn node_declares_custom_properties(&self, node: StyleNodeID) -> bool {
        if !self.any_custom_property_is_declared() {
            return false;
        }
        if !self.facts.element_custom_declarations(node).is_empty() {
            return true;
        }
        matches!(
            self.try_for_each_element_match(node, |rule, _, _, _| {
                if self.program.custom_declarations_of(rule).is_empty() {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            }),
            Some(ControlFlow::Break(()))
        )
    }

    /// Whether a reaction on the node may move its custom-property environment, which its
    /// descendants inherit: it holds an environment of its own, or its cascade declares custom
    /// properties now. A node whose environment is its parent's and whose cascade declares none
    /// keeps the parent's whatever its reaction computes.
    pub(super) fn node_environment_may_move(&self, node: StyleNodeID) -> bool {
        let own = self.computed_group_sets.custom_property_environment_identity(node);
        let parent = self.tree.flat_tree_parent(node).map_or(Some(0), |parent| {
            self.computed_group_sets.custom_property_environment_identity(parent)
        });
        own != parent || self.node_declares_custom_properties(node)
    }

    /// Whether the node's style reads custom properties: its cascade declares some, or a winner
    /// of its state was written with a substitution. A node whose reads are unknown reads.
    pub fn node_style_reads_custom_properties(&mut self, node: StyleNodeID) -> bool {
        // Published substitution usage also includes the pseudo styles C++ computes, whose
        // reads are not represented by the element's own winner state.
        if self.facts.uses_unnamed_custom_properties(node) || self.node_declares_custom_properties(node) {
            return true;
        }
        let Lookup::Known((_, state)) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        else {
            return true;
        };
        self.state_has_substitutions(node, state)
    }

    /// Whether the node's winner inventory is complete once custom properties are set aside: the
    /// engine computes an environment from those itself, and a rule declaring them is otherwise as
    /// complete as any. A pseudo-element's rules keep the strict reading, since a pseudo-element's
    /// environment is still C++'s to compute.
    pub(super) fn cascade_winners_are_complete_but_for_custom_properties(&self, node: StyleNodeID) -> bool {
        if !ElementDeclarationKind::ALL.iter().all(|&kind| {
            self.facts
                .element_declarations_are_complete_but_for_custom_properties(node, kind)
        }) {
            return false;
        }
        // A rule deciding from another tree scope orders by its context like any other; the
        // record path reads the winners the cascade holds for it, whichever scope it decided from.
        let rule_is_complete = |rule: RuleID, _tree_scope: TreeScopeID, pseudo: bool| {
            !self.program.rule_is_gated_by_container_query(rule)
                && if pseudo {
                    self.program.declarations_are_complete_for(rule)
                } else {
                    self.program.declarations_are_complete_but_for_custom_properties(rule)
                }
        };
        if let Some((published, answer)) = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        ) && let Some(matches) = published.matches_for(answer)
        {
            return matches
                .iter()
                .all(|entry| rule_is_complete(entry.rule, entry.tree_scope, entry.pseudo_element.is_some()));
        }
        let Lookup::Known(answer) = self.retained_match_answer(node) else {
            return false;
        };
        answer.iter().all(|rule_match| {
            let entry = &self.programs.get(rule_match.program).entries()[rule_match.entry as usize];
            rule_is_complete(rule_match.rule, rule_match.tree_scope, entry.pseudo_element.is_some())
        })
    }

    /// The custom properties the node's cascade decides, each with its winning declaration and
    /// the value it was written with, in the order the C++ cascade lists them: by first
    /// appearance, applying blocks from the lowest priority up, a later declaration of a name
    /// replacing the earlier in place. `None` when the node has no answer to cascade from, or a
    /// declaration arrived without its written value.
    fn cascaded_custom_declarations(
        &self,
        node: StyleNodeID,
    ) -> Option<Vec<(CustomDeclaration, RetainedStyleValueData)>> {
        self.cascaded_custom_declarations_of(node, None)
    }

    /// What `cascaded_custom_declarations` says of the element, for one of its pseudo-elements:
    /// its rules alone, since a pseudo-element has no declarations of its own.
    fn cascaded_custom_declarations_of(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
    ) -> Option<Vec<(CustomDeclaration, RetainedStyleValueData)>> {
        self.cascade_custom_declarations(node, pseudo, None)
    }

    /// What a pseudo-element's custom declarations cascade to, from the matches being published
    /// for it rather than from a published answer. `None` when a declaration arrived without its
    /// written value.
    pub(super) fn cascaded_pseudo_custom_declarations_in(
        &self,
        node: StyleNodeID,
        matches: &[RuleMatch],
        pseudo: tree::PseudoElementTarget,
    ) -> Option<Vec<CustomDeclaration>> {
        let kind = u8::try_from(pseudo.kind.0).ok()?;
        let cascaded = self.cascade_custom_declarations(node, Some(kind), Some(matches))?;
        Some(cascaded.into_iter().map(|(declared, _)| declared).collect())
    }

    fn cascade_custom_declarations(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        matches: Option<&[RuleMatch]>,
    ) -> Option<Vec<(CustomDeclaration, RetainedStyleValueData)>> {
        struct Candidate<'a> {
            priority: CascadePriority,
            stratum: CascadeStratum,
            declared: CustomDeclaration,
            written: &'a RetainedStyleValueData,
        }

        let mut candidates = Vec::new();
        let mut visit = |rule: RuleID, tree_scope: TreeScopeID, specificity: Specificity, scope_proximity: u32| {
            let declared = self.program.custom_declarations_of(rule);
            if declared.is_empty() {
                return ControlFlow::Continue(());
            }
            let written = self.program.custom_written_values_of(rule);
            if written.len() != declared.len() {
                return ControlFlow::Break(());
            }
            let mut priority_and_stratum_by_importance = [None; 2];
            for (&declared, written) in declared.iter().zip(written) {
                let (priority, stratum) = *priority_and_stratum_by_importance[declared.important as usize]
                    .get_or_insert_with(|| {
                        (
                            self.cascade_priority_of(
                                rule,
                                tree_scope,
                                specificity,
                                scope_proximity,
                                declared.important,
                            ),
                            self.cascade_stratum_of(rule, tree_scope, declared.important),
                        )
                    });
                candidates.push(Candidate {
                    priority,
                    stratum,
                    declared,
                    written,
                });
            }
            ControlFlow::Continue(())
        };
        let result = match matches {
            Some(matches) => {
                let wanted = pseudo.map(u16::from);
                let mut result = ControlFlow::Continue(());
                for entry in matches
                    .iter()
                    .filter(|entry| entry.pseudo_element.map(|target| target.kind.0) == wanted)
                {
                    result = visit(entry.rule, entry.tree_scope, entry.specificity, entry.scope_proximity);
                    if result.is_break() {
                        break;
                    }
                }
                result
            }
            None => self.try_for_each_match(node, pseudo, &mut visit)?,
        };
        if result.is_break() {
            return None;
        }
        let (declared, written) = match pseudo {
            None => (
                self.facts.element_custom_declarations(node),
                self.facts.element_custom_written_values(node),
            ),
            Some(_) => (&[][..], &[][..]),
        };
        if written.len() != declared.len() {
            return None;
        }
        let mut priority_and_stratum_by_importance = [None; 2];
        for (&declared, written) in declared.iter().zip(written) {
            let (priority, stratum) = *priority_and_stratum_by_importance[declared.important as usize]
                .get_or_insert_with(|| {
                    (
                        self.element_cascade_priority(node, ElementDeclarationKind::InlineStyle, declared.important),
                        self.element_cascade_stratum(node, ElementDeclarationKind::InlineStyle, declared.important),
                    )
                });
            candidates.push(Candidate {
                priority,
                stratum,
                declared,
                written,
            });
        }
        if let [candidate] = candidates.as_slice() {
            return Some(if candidate.stratum.ceiling(candidate.declared.operator).is_none() {
                vec![(candidate.declared, candidate.written.clone_retained())]
            } else {
                Vec::new()
            });
        }
        // Keep insertion order for declarations with equal cascade priority.
        candidates.sort_by_key(|candidate| candidate.priority);
        let mut name_indices = HashMap::default();
        let mut candidates_by_name: Vec<Vec<Candidate>> = Vec::new();
        for candidate in candidates {
            let index = *name_indices.entry(candidate.declared.name).or_insert_with(|| {
                candidates_by_name.push(Vec::new());
                candidates_by_name.len() - 1
            });
            candidates_by_name[index].push(candidate);
        }
        let mut cascaded = Vec::with_capacity(candidates_by_name.len());
        let mut ceilings = Vec::new();
        for candidates in candidates_by_name {
            ceilings.clear();
            for candidate in candidates.into_iter().rev() {
                if !ceilings.iter().all(|&ceiling| candidate.stratum.is_below(ceiling)) {
                    continue;
                }
                let Some(ceiling) = candidate.stratum.ceiling(candidate.declared.operator) else {
                    cascaded.push((candidate.declared, candidate.written.clone_retained()));
                    break;
                };
                ceilings.push(ceiling);
            }
        }
        Some(cascaded)
    }

    fn environment_inputs(
        parent: u64,
        registration_generation: u64,
        cascaded: &[(CustomDeclaration, RetainedStyleValueData)],
    ) -> custom_property_environments::EnvironmentInputs {
        custom_property_environments::EnvironmentInputs {
            parent,
            registration_generation,
            cascaded: cascaded
                .iter()
                .map(|(declared, written)| CascadedCustomProperty {
                    name: declared.name,
                    important: declared.important,
                    written_value: written.pointer() as usize,
                })
                .collect(),
        }
    }

    /// Remember the environment C++ resolved for a node's custom declarations, so a later engine
    /// derivation of the node, or of an element alike in its declarations, takes that environment
    /// rather than resolving an equal one under an identity of its own.
    pub(super) fn remember_cpp_custom_property_environment(&mut self, node: StyleNodeID, environment: u64) {
        if environment == 0 || environment & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0 {
            return;
        }
        let Some(inputs) = self.document_style_computation_inputs else {
            return;
        };
        if !self.node_declares_custom_properties(node) {
            return;
        }
        let Some(cascaded) = self.cascaded_custom_declarations(node) else {
            return;
        };
        if cascaded.is_empty() {
            return;
        }
        if cascaded
            .iter()
            .any(|(_, written)| !custom_property_value_is_engine_resolvable(written.data()))
        {
            return;
        }
        let parent_environment = match self.tree.flat_tree_parent(node) {
            Some(parent) => match self.computed_group_sets.custom_property_environment_identity(parent) {
                Some(parent_environment) => parent_environment,
                None => return,
            },
            None => 0,
        };
        let key = Self::environment_inputs(
            parent_environment,
            inputs.custom_property_registration_generation,
            &cascaded,
        );
        if self.custom_property_environments.memoized(&key).is_none() {
            let written_values = cascaded.into_iter().map(|(_, written)| written).collect();
            self.custom_property_environments
                .remember(key, environment, written_values);
        }
    }

    /// The environment of a node the engine computes a record for: the one it inherits when its
    /// cascade declares no custom property, else what its declarations resolve to over that one.
    /// `None` when the environment is C++'s to compute: a registered name, a substitution the
    /// engine does not resolve, or an inherited environment the engine holds no store for.
    pub(super) fn engine_custom_property_environment(
        &mut self,
        node: StyleNodeID,
        parent_environment: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        counters: &mut Counters,
    ) -> Option<u64> {
        self.engine_custom_property_environment_of(node, None, parent_environment, inputs, counters)
    }

    /// What `engine_custom_property_environment` says of the element, for one of its
    /// pseudo-elements over the element's own environment.
    pub(super) fn engine_custom_property_environment_of(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        parent_environment: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        counters: &mut Counters,
    ) -> Option<u64> {
        if !self.any_custom_property_is_declared() {
            return Some(parent_environment);
        }
        let cascaded = self.cascaded_custom_declarations_of(node, pseudo)?;
        if cascaded.is_empty() {
            return Some(parent_environment);
        }
        let registry = inputs.custom_property_registry;
        if registry.is_none() {
            counters.bump(Counter::EngineCustomPropertyEnvironmentBails);
            return None;
        }
        let registry_ref = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
        if registry_ref.has_registrations() {
            counters.bump(Counter::EngineCustomPropertyEnvironmentBails);
            return None;
        }
        let key = Self::environment_inputs(
            parent_environment,
            inputs.custom_property_registration_generation,
            &cascaded,
        );
        // An environment C++ resolved for an element alike in its declarations is one C++ holds
        // for that element: a pseudo-element's own is resolved by the engine, and C++'s stays
        // remembered for the elements.
        let memoized = self.custom_property_environments.memoized(&key);
        let keeps_cpp_environment = pseudo.is_some()
            && memoized.is_some_and(|identity| {
                identity != parent_environment
                    && identity & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT == 0
            });
        if let Some(identity) = memoized.filter(|_| !keeps_cpp_environment) {
            counters.bump(Counter::EngineCustomPropertyEnvironmentMemoHits);
            return Some(identity);
        }
        let parent_store = match parent_environment {
            0 => std::ptr::null(),
            identity => {
                let Some(store) = self.custom_property_environments.store(identity) else {
                    counters.bump(Counter::EngineCustomPropertyEnvironmentBails);
                    return None;
                };
                store
            }
        };
        let parent = unsafe { parent_store.cast::<CustomPropertyStore>().as_ref() };
        let mut values = Vec::with_capacity(cascaded.len());
        for (declared, value) in &cascaded {
            let Some(name) = self.custom_property_environments.name(declared.name) else {
                counters.bump(Counter::EngineCustomPropertyEnvironmentBails);
                return None;
            };
            if name.raw.raw() == 0 || !custom_property_value_is_engine_resolvable(value.data()) {
                counters.bump(Counter::EngineCustomPropertyEnvironmentBails);
                return None;
            }
            // A value the parent already holds, by identity, declares nothing new.
            if parent.is_some_and(|parent| parent.value_is_identical(name.raw.raw(), value.pointer().cast())) {
                continue;
            }
            values.push((
                name.raw.raw(),
                name.text.clone(),
                declared.important,
                value.pointer().cast(),
            ));
        }
        if values.is_empty() {
            let written_values = cascaded.into_iter().map(|(_, written)| written).collect();
            if !keeps_cpp_environment {
                self.custom_property_environments
                    .remember(key, parent_environment, written_values);
            }
            return Some(parent_environment);
        }
        // SAFETY: The parent store is live for as long as a record names its environment, and the
        // values are the program's interned values, live for the call.
        let cascaded_store = unsafe { CustomPropertyStore::cascaded_child(parent_store, values) };
        let mut random_function_index = 0_usize;
        let parse_context = registry_ref.parse_context(&mut random_function_index);
        let resolution_context =
            engine_resolution_context(&parse_context, cascaded_store, parent_store, registry.as_pointer());
        let drive = FfiCustomPropertyDriveInput {
            store: cascaded_store,
            resolved_parent_store: parent_store,
            reuse_resolved_parent_if_empty: !parent_store.is_null(),
            resolution_context: &raw const resolution_context,
            finalization_environment: std::ptr::null(),
            finalization_color_scheme: 0,
        };
        // SAFETY: Every pointer the drive reads is live for the call, and the finalizer replaces
        // each output with one transferred reference.
        let resolved = unsafe { drive_custom_property_resolution(&drive) };
        // The resolved values live in the store; the listing transfers references of its own.
        let properties = match resolved.count {
            0 => &[],
            count => unsafe { std::slice::from_raw_parts(resolved.properties, count) },
        };
        for property in properties {
            unsafe { release_style_value(property.data.cast()) };
        }
        unsafe { destroy_resolved_custom_properties(resolved.storage, resolved.count) };
        unsafe { Arc::decrement_strong_count(cascaded_store.cast::<CustomPropertyStore>()) };
        counters.bump(Counter::EngineCustomPropertyEnvironmentsResolved);
        let identity = if resolved.rust_store.is_null() {
            parent_environment
        } else {
            unsafe {
                self.custom_property_environments
                    .adopt_engine_environment(resolved.rust_store, parent_environment)
            }
        };
        let written_values = cascaded.into_iter().map(|(_, written)| written).collect();
        if !keeps_cpp_environment {
            self.custom_property_environments
                .remember(key, identity, written_values);
        }
        Some(identity)
    }

    /// What a written value with `var()` references substitutes to for a property under an
    /// environment, parsed as the property's value: what the C++ cascade computes for the
    /// declaration, memoized by the written value. `None` when the value holds a substitution
    /// the engine does not resolve, or the environment is one the engine holds no store for.
    pub(super) fn substitute_written_value(
        environments: &mut custom_property_environments::CustomPropertyEnvironments,
        inputs: Option<bridge::FfiDocumentStyleComputationInputs>,
        environment: u64,
        property: u16,
        written: RetainedStyleValueData,
        counters: &mut Counters,
    ) -> Option<RetainedStyleValueData> {
        if !custom_property_value_is_engine_resolvable(written.data()) {
            counters.bump(Counter::EngineComputedRecordBailSubstitution);
            return None;
        }
        let Some(inputs) = inputs else {
            counters.bump(Counter::EngineComputedRecordBailSubstitution);
            return None;
        };
        let registry = inputs.custom_property_registry;
        if registry.is_none() {
            counters.bump(Counter::EngineComputedRecordBailSubstitution);
            return None;
        }
        if let Some(value) = environments.substitution(&written, property, environment) {
            counters.bump(Counter::EngineComputedRecordSubstitutionMemoHits);
            return Some(value);
        }
        let store = match environment {
            0 => std::ptr::null(),
            identity => {
                let Some(store) = environments.store(identity) else {
                    counters.bump(Counter::EngineComputedRecordBailSubstitution);
                    return None;
                };
                store
            }
        };
        let value = substitute_written_value_against_store(store, inputs, property, &written, counters)?;
        counters.bump(Counter::EngineComputedRecordSubstitutions);
        environments.remember_substitution(written, property, environment, value.clone_retained());
        Some(value)
    }
}

/// What a written value with `var()` references substitutes to for a property against a resolved
/// custom-property store, parsed as the property's value: what the C++ cascade computes for the
/// declaration. Memo-free, for a caller that holds the store rather than the environment table.
/// `None` when the value holds a substitution the engine does not resolve, or the substituted
/// source is a grammar the Rust parser does not handle.
pub(crate) fn substitute_written_value_against_store(
    store: *const c_void,
    inputs: bridge::FfiDocumentStyleComputationInputs,
    property: u16,
    written: &RetainedStyleValueData,
    counters: &mut Counters,
) -> Option<RetainedStyleValueData> {
    if !custom_property_value_is_engine_resolvable(written.data()) {
        counters.bump(Counter::EngineComputedRecordBailSubstitution);
        return None;
    }
    let registry = inputs.custom_property_registry;
    if registry.is_none() {
        counters.bump(Counter::EngineComputedRecordBailSubstitution);
        return None;
    }
    let registry_ref = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
    let mut random_function_index = 0_usize;
    let mut parse_context = registry_ref.parse_context(&mut random_function_index);
    parse_context.in_quirks_mode = inputs.in_quirks_mode;
    let Some(mut resolution_environment) = (unsafe {
        prepare_var_resolution_environment(std::ptr::null(), 0, std::ptr::null(), 0, 0, std::ptr::null(), 0)
    }) else {
        counters.bump(Counter::EngineComputedRecordBailSubstitution);
        return None;
    };
    // SAFETY: The store is live while a record names its environment, and the written value
    // is retained by the declaration that carries it.
    let resolution = unsafe {
        crate::css::custom_properties::resolve_vars(
            store,
            std::ptr::null(),
            registry.as_pointer(),
            Some(&parse_context),
            None,
            None,
            property,
            FfiUtf16View {
                ascii: std::ptr::null(),
                utf16: std::ptr::null(),
                length: 0,
            },
            written.pointer().cast(),
            &mut resolution_environment,
            false,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            None,
        )
    };
    // The substituted source parses as the C++ cascade parses it: without callbacks first,
    // then with the parse context's. A grammar the Rust parser does not handle parses in C++;
    // the value is C++'s to compute.
    let value = match resolution {
        NativeVarResolution::Resolved {
            source,
            contains_attr_tainted_values,
        } => {
            let CallbackFreeParseOutcome { outcome, source } =
                parse_substituted_without_callbacks(&parse_context, property, source, contains_attr_tainted_values);
            let outcome = match outcome {
                ParseOutcome::NotHandled => {
                    parse_substituted_source(&parse_context, property, &source, contains_attr_tainted_values)
                }
                outcome => outcome,
            };
            match outcome {
                ParseOutcome::Parsed(value) => unsafe {
                    RetainedStyleValueData::from_retained_pointer(std::sync::Arc::into_raw(value))
                },
                ParseOutcome::Invalid => RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid),
                ParseOutcome::NotHandled => {
                    counters.bump(Counter::EngineComputedRecordBailSubstitution);
                    return None;
                }
            }
        }
        NativeVarResolution::Invalid => RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid),
        NativeVarResolution::NotHandled => {
            counters.bump(Counter::EngineComputedRecordBailSubstitution);
            return None;
        }
    };
    Some(value)
}
