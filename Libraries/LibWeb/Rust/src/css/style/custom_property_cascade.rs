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

use super::publication::{Drive, Suspension, Unanswered};
use super::*;
use crate::css::cascaded_properties::{
    CallbackFreeParseOutcome, FfiCascadeResolutionContext, FfiCustomPropertyDriveInput,
    custom_property_value_is_callback_free as custom_property_value_is_engine_resolvable,
    destroy_resolved_custom_properties, drive_custom_property_resolution, parse_substituted_source,
    parse_substituted_without_callbacks,
};
use crate::css::custom_properties::{
    CustomPropertyRegistry, CustomPropertyStore, FfiSubstitutionFunctionDeclaration, FfiSubstitutionFunctionDefinition,
    FfiSubstitutionFunctionVisibility, NativeVarResolution, prepare_var_resolution_environment,
};
use crate::css::ffi_support::FfiUtf16View;
use crate::css::parser::value_parser::ParseOutcome;
use crate::css::rule::CompiledFunction;
use crate::css::style_value::{RetainedStyleValueData, StyleValueData, release_style_value};
use custom_property_environments::{CascadedCustomProperty, CustomPropertyName};

/// A transaction's media features copied from the host. The length context's only output pointer
/// is cleared before retaining it, so neither part borrows the style update's stack.
#[derive(Default)]
pub(super) struct DocumentMediaSnapshot {
    values: Vec<crate::css::parser::query_parser::FfiMediaFeatureValue>,
    length: Option<crate::css::style_compute::FfiLengthResolutionContext>,
}

// The copied length context contains no writable pointer after `take_from` clears it.
unsafe impl Send for DocumentMediaSnapshot {}
unsafe impl Sync for DocumentMediaSnapshot {}

/// The host's parsed @function table, frozen before a transaction. Compiled functions own their
/// declarations; the engine decides which conditional declarations apply to each subject.
#[derive(Default)]
pub(super) struct DocumentFunctionSnapshot {
    definitions: HashMap<u64, (Arc<CompiledFunction>, usize)>,
    visibilities: Vec<FfiSubstitutionFunctionVisibility>,
    caller_scopes: HashMap<u32, usize>,
}

impl DocumentFunctionSnapshot {
    unsafe fn publish(
        &mut self,
        function: *const CompiledFunction,
        caller_scope: usize,
        definition_scope: usize,
        tree_scope: u32,
    ) {
        let Some(function_ref) = (unsafe { function.as_ref() }) else {
            return;
        };
        self.definitions.entry(function_ref.identity).or_insert_with(|| {
            unsafe { Arc::increment_strong_count(function) };
            (unsafe { Arc::from_raw(function) }, definition_scope)
        });
        self.visibilities.push(FfiSubstitutionFunctionVisibility {
            caller_scope_identity: caller_scope,
            function_identity: function_ref.identity,
        });
        if tree_scope != u32::MAX {
            self.caller_scopes.insert(tree_scope, caller_scope);
        }
    }
}

pub(super) struct PreparedCustomFunctions {
    _declarations: Vec<Vec<FfiSubstitutionFunctionDeclaration>>,
    definitions: Vec<FfiSubstitutionFunctionDefinition>,
    visibilities: Vec<FfiSubstitutionFunctionVisibility>,
    caller_scope: usize,
    reads_attributes: bool,
}

#[unsafe(no_mangle)]
unsafe extern "C" fn style_engine_reset_custom_functions(engine: *mut c_void) {
    crate::stage_thread::join_frame_for_style_engine_entrance(engine, "style_engine_reset_custom_functions");
    let engine = unsafe { &mut *engine.cast::<StyleEngine>() };
    engine.document_function_snapshot = DocumentFunctionSnapshot::default();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn style_engine_publish_custom_function(
    engine: *mut c_void,
    function: *const CompiledFunction,
    caller_scope: usize,
    definition_scope: usize,
    tree_scope: u32,
) {
    crate::stage_thread::join_frame_for_style_engine_entrance(engine, "style_engine_publish_custom_function");
    let engine = unsafe { &mut *engine.cast::<StyleEngine>() };
    unsafe {
        engine
            .document_function_snapshot
            .publish(function, caller_scope, definition_scope, tree_scope);
    }
}

impl RetainedState {
    /// The custom functions visible from the node's scope, with the declarations each one's
    /// conditions select for it. The host publishes a definition beside every visibility that
    /// names it; a condition on a query container the engine cannot decide selects nothing, as a
    /// container query with no eligible container matches nothing.
    fn prepare_custom_functions(&mut self, node: StyleNodeID, pseudo: Option<u8>) -> PreparedCustomFunctions {
        let snapshot = &self.document_function_snapshot;
        // The host publishes a caller scope when that scope can see a definition. With no
        // visible definition, an empty registry still resolves an unknown function's fallback.
        let caller_scope = snapshot
            .caller_scopes
            .get(&self.tree.tree_scope(node).0)
            .copied()
            .unwrap_or(0);
        let mut scopes = vec![caller_scope];
        let mut identities = Vec::new();
        let mut seen = HashSet::default();
        let mut index = 0;
        while index < scopes.len() {
            let scope = scopes[index];
            for visibility in &snapshot.visibilities {
                if visibility.caller_scope_identity != scope || !seen.insert(visibility.function_identity) {
                    continue;
                }
                let Some((_, definition_scope)) = snapshot.definitions.get(&visibility.function_identity) else {
                    debug_assert!(false, "a visible custom function was published without its definition");
                    continue;
                };
                identities.push(visibility.function_identity);
                if !scopes.contains(definition_scope) {
                    scopes.push(*definition_scope);
                }
            }
            index += 1;
        }
        let compiled = identities
            .iter()
            .map(|identity| {
                let (function, scope) = &snapshot.definitions[identity];
                (function.clone(), *scope)
            })
            .collect::<Vec<_>>();
        let media_environment = self.document_media_snapshot.as_ffi();
        let media_environment = unsafe { media_environment.borrow() };
        let visibilities = snapshot
            .visibilities
            .iter()
            .filter(|visibility| scopes.contains(&visibility.caller_scope_identity))
            .map(|visibility| FfiSubstitutionFunctionVisibility {
                caller_scope_identity: visibility.caller_scope_identity,
                function_identity: visibility.function_identity,
            })
            .collect();
        let mut declarations = Vec::with_capacity(compiled.len());
        let mut container_effects = super::container_queries::ContainerVerdict::default();
        let mut reads_attributes = false;
        for (function, _) in &compiled {
            let mut function_declarations = Vec::new();
            for input in &function.inputs {
                if input.media.iter().any(|list| {
                    !list.queries.is_empty() && !list.queries.iter().any(|query| query.matches_media(media_environment))
                }) {
                    continue;
                }
                if !input.containers.is_empty() {
                    let Some(verdict) = self.function_container_verdict(node, pseudo.is_some(), &input.containers)
                    else {
                        continue;
                    };
                    container_effects.depends_on_size |= verdict.depends_on_size;
                    container_effects.depends_on_style |= verdict.depends_on_style;
                    container_effects.effects.extend(verdict.effects);
                    if !verdict.matches {
                        continue;
                    }
                }
                for descriptor in &input.declarations.descriptors {
                    reads_attributes |= matches!(
                        descriptor.value.as_ref(),
                        StyleValueData::Unresolved {
                            presence_attr: true,
                            ..
                        }
                    );
                    let name = descriptor.name.units();
                    function_declarations.push(FfiSubstitutionFunctionDeclaration {
                        name: FfiUtf16View {
                            ascii: std::ptr::null(),
                            utf16: name.as_ptr(),
                            length: name.len(),
                        },
                        data: Arc::as_ptr(&descriptor.value).cast(),
                    });
                }
            }
            declarations.push(function_declarations);
        }
        if container_effects.depends_on_size
            || container_effects.depends_on_style
            || !container_effects.effects.is_empty()
        {
            self.note_container_effects_for_host(node, &container_effects);
        }
        let definitions = compiled
            .iter()
            .zip(&declarations)
            .map(|((function, scope), declarations)| FfiSubstitutionFunctionDefinition {
                identity: function.identity,
                scope_identity: *scope,
                signature: Arc::as_ptr(&function.signature).cast(),
                declarations: declarations.as_ptr(),
                declaration_count: declarations.len(),
            })
            .collect();
        PreparedCustomFunctions {
            _declarations: declarations,
            definitions,
            visibilities,
            caller_scope,
            reads_attributes,
        }
    }
}

impl DocumentMediaSnapshot {
    pub(super) unsafe fn take_from(inputs: &mut bridge::FfiDocumentStyleComputationInputs) -> Self {
        let values = if inputs.media_feature_value_count == 0 {
            Vec::new()
        } else {
            unsafe {
                std::slice::from_raw_parts(
                    inputs.media_feature_values.as_pointer().cast(),
                    inputs.media_feature_value_count,
                )
                .to_vec()
            }
        };
        let length = unsafe {
            inputs
                .media_length_resolution_context
                .as_pointer()
                .cast::<crate::css::style_compute::FfiLengthResolutionContext>()
                .as_ref()
                .copied()
        }
        .map(|mut context| {
            context.resolved_viewport_relative_length = std::ptr::null_mut();
            context
        });
        inputs.media_feature_values = bridge::FfiHostHandle { address: 0 };
        inputs.media_feature_value_count = 0;
        inputs.media_length_resolution_context = bridge::FfiHostHandle { address: 0 };
        Self { values, length }
    }

    fn as_ffi(&self) -> crate::css::parser::query_parser::FfiMediaEnvironment {
        crate::css::parser::query_parser::FfiMediaEnvironment {
            values: self.values.as_ptr(),
            value_count: self.values.len(),
            length_resolution_context: self
                .length
                .as_ref()
                .map_or(std::ptr::null(), |length| std::ptr::from_ref(length).cast()),
        }
    }
}

/// The resolution context the engine substitutes under: the stores alone, with no callback into
/// C++ - what the engine cannot resolve without one is left to C++ before this is built.
#[expect(
    clippy::too_many_arguments,
    reason = "the FFI context borrows independent resolution inputs"
)]
fn engine_resolution_context(
    parse_context: &crate::css::parser::value_parser::ParseContext,
    store: *const c_void,
    inheritance_store: *const c_void,
    registry: *const c_void,
    length: *const crate::css::style_compute::FfiLengthResolutionContext,
    attributes: &[crate::css::custom_properties::FfiSubstitutionAttribute],
    attribute_names_are_ascii_case_insensitive: bool,
    media_environment: *const crate::css::parser::query_parser::FfiMediaEnvironment,
) -> FfiCascadeResolutionContext {
    FfiCascadeResolutionContext {
        parse_context: std::ptr::from_ref(parse_context).cast(),
        media_environment: media_environment.cast(),
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
        attributes: attributes.as_ptr(),
        attribute_count: attributes.len(),
        attribute_names_are_ascii_case_insensitive,
        custom_functions: std::ptr::null(),
        custom_function_count: 0,
        custom_function_scope_identity: 0,
        custom_function_visibilities: std::ptr::null(),
        custom_function_visibility_count: 0,
        style_query_length_resolution_context: length.cast(),
        style_query_dependencies: std::ptr::null_mut(),
    }
}

/// What a registered custom property's value is computed against: the element's own font metrics
/// and viewport, as the font phase leaves them, and the colour scheme its table carries. A row
/// that cannot offer these leaves every registered declaration to the host.
pub(super) struct RegisteredValueContext {
    pub length: crate::css::style_compute::FfiLengthResolutionContext,
    pub color_scheme: u8,
}

// The context retained across a font request carries only copied metrics and a
// null output pointer. Every constructor of this value sets that pointer to
// null before the context enters batch scratch, so moving it between workers
// cannot move an aliased output location.
unsafe impl Send for RegisteredValueContext {}

/// Whether a token stream is a substitution the engine can resolve with the published inputs.
pub(super) fn value_is_engine_resolvable_substitution(value: &StyleValueData) -> bool {
    if let StyleValueData::PendingSubstitution {
        original_shorthand_value,
    } = value
    {
        return value_is_engine_resolvable_substitution(original_shorthand_value.data());
    }
    (matches!(value, StyleValueData::Unresolved { .. }) && custom_property_value_is_engine_resolvable(value))
        || matches!(
            value,
            StyleValueData::Unresolved {
                presence_inherit: true,
                ..
            } | StyleValueData::Unresolved {
                presence_dashed_function: true,
                ..
            } | StyleValueData::Unresolved { presence_if: true, .. }
        )
}

impl RetainedState {
    pub(super) fn declares_registered_custom_property(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> bool {
        let registry = inputs.custom_property_registry;
        if registry.is_none() {
            return false;
        }
        let registry = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
        if !registry.has_registrations() {
            return false;
        }
        self.cascaded_custom_declarations_of(node, pseudo)
            .is_some_and(|declarations| self.declarations_name_a_registered_custom_property(&declarations, inputs))
    }

    pub(super) fn declarations_name_a_registered_custom_property(
        &self,
        declarations: &[(CustomDeclaration, RetainedStyleValueData)],
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> bool {
        let registry = inputs.custom_property_registry;
        if registry.is_none() {
            return false;
        }
        let registry = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
        registry.has_registrations()
            && declarations.iter().any(|(declared, _)| {
                self.custom_property_environments
                    .name(declared.name)
                    .is_some_and(|name| registry.registration_facts(&name.text).is_some())
            })
    }
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
    /// The matches declaring custom properties in the answer a transaction publishes for the node,
    /// read before that answer is installed: the answers the lookups below find are the installed
    /// ones, which a node published in this transaction does not have yet, or has from before.
    pub(super) fn batch_custom_property_matches_of(
        &self,
        published: &PublishedMatchAnswers,
        answer: &PublishedMatchAnswer,
    ) -> Option<Vec<BatchCustomPropertyMatch>> {
        let mut matches = Vec::new();
        let mut push = |rule: RuleID,
                        tree_scope: TreeScopeID,
                        specificity: Specificity,
                        scope_proximity: u32,
                        pseudo: Option<u16>| {
            if !self.program.custom_declarations_of(rule).is_empty() {
                matches.push(BatchCustomPropertyMatch {
                    rule,
                    tree_scope,
                    specificity,
                    scope_proximity,
                    pseudo,
                });
            }
        };
        if let Some(published_matches) = published.matches_for(answer) {
            for entry in published_matches {
                push(
                    entry.rule,
                    entry.tree_scope,
                    entry.specificity,
                    entry.scope_proximity,
                    entry.pseudo_element.map(|target| target.kind.0),
                );
            }
        } else {
            for rule_match in self.match_answers.answer(answer.cascade_input?)?.iter() {
                let entry = &self.programs.get(rule_match.program).entries()[rule_match.entry as usize];
                push(
                    rule_match.rule,
                    rule_match.tree_scope,
                    entry.specificity,
                    rule_match.scope_proximity,
                    entry.pseudo_element.map(|target| target.kind.0),
                );
            }
        }
        Some(matches)
    }

    fn try_for_each_match(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        mut visit: impl FnMut(RuleID, TreeScopeID, Specificity, u32) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        let wanted =
            |target: Option<tree::PseudoElementTarget>| target.map(|target| target.kind.0) == pseudo.map(u16::from);
        if let Some(matches) = self.batch_custom_property_matches.get(&node) {
            for entry in matches.iter().filter(|entry| entry.pseudo == pseudo.map(u16::from)) {
                if visit(entry.rule, entry.tree_scope, entry.specificity, entry.scope_proximity).is_break() {
                    return Some(ControlFlow::Break(()));
                }
            }
            return Some(ControlFlow::Continue(()));
        }
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
        // An answer published by its identity alone names the matches the catalog holds for it;
        // the answer the node retains from before is not the one it is being published with.
        let published_identity = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        )
        .map(|(_, answer)| answer.cascade_input);
        let answer = match published_identity {
            Some(identity) => identity.and_then(|identity| self.match_answers.answer(identity))?,
            None => match self.retained_match_answer(node) {
                Lookup::Known(answer) => answer,
                _ => return None,
            },
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
    pub(super) fn any_custom_property_is_declared(&self) -> bool {
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

    /// The environment a node holds over its parent's as the parent holds it now: the parent's,
    /// inheritable, for a node declaring no custom property, and its own declarations resolved
    /// over it where the parent's moved. `None` where the node's may stand as it is, or where
    /// the engine cannot tell what it declares; a suspension where resolving its declarations
    /// waits on a request.
    pub(super) fn environment_over_current_parent(
        &mut self,
        node: StyleNodeID,
        parent_environment: u64,
        parent_moved: bool,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> Result<Option<u64>, Suspension> {
        if !self.any_custom_property_is_declared() {
            return Ok(Some(parent_environment));
        }
        let declares = !self.facts.element_custom_declarations(node).is_empty()
            || self.try_for_each_element_match(node, |rule, _, _, _| {
                if self.program.custom_declarations_of(rule).is_empty() {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            }) != Some(ControlFlow::Continue(()));
        if !declares {
            return Ok(Some(
                self.inheritable_custom_property_environment(parent_environment, inputs),
            ));
        }
        if !parent_moved {
            return Ok(None);
        }
        match self.engine_custom_property_environment(node, parent_environment, inputs, None, &mut Counters::default())
        {
            Ok(environment) => Ok(Some(environment)),
            Err(Unanswered::Suspended(suspension)) => Err(suspension),
        }
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
    /// complete as any, for the element and for each of its pseudo-elements alike.
    pub(super) fn cascade_winners_are_complete_but_for_custom_properties(&self, node: StyleNodeID) -> bool {
        if let Some(&complete) = self.batch_answers_complete_but_for_custom_properties.get(&node) {
            return complete;
        }
        if let Some((published, answer)) = Self::published_answer_lookup(
            &self.published_match_answers,
            self.batch_matching_traversal.as_deref(),
            node,
        ) && let Some(matches) = published.matches_for(answer)
        {
            return matches.iter().all(|entry| {
                self.match_is_complete_but_for_custom_properties(
                    node,
                    entry.rule,
                    entry.tree_scope,
                    entry.pseudo_element.is_some(),
                )
            });
        }
        let Lookup::Known(answer) = self.retained_match_answer(node) else {
            return false;
        };
        self.retained_matches_are_complete_but_for_custom_properties(node, answer)
    }

    /// What `cascade_winners_are_complete_but_for_custom_properties` says of the answer a
    /// transaction publishes for the node, read before that answer is installed. `None` when the
    /// answer holds neither its matches nor an identity the catalog materializes.
    pub(super) fn answer_is_complete_but_for_custom_properties(
        &self,
        node: StyleNodeID,
        published: &PublishedMatchAnswers,
        answer: &PublishedMatchAnswer,
    ) -> Option<bool> {
        if let Some(matches) = published.matches_for(answer) {
            return Some(matches.iter().all(|entry| {
                self.match_is_complete_but_for_custom_properties(
                    node,
                    entry.rule,
                    entry.tree_scope,
                    entry.pseudo_element.is_some(),
                )
            }));
        }
        let matches = self.match_answers.answer(answer.cascade_input?)?;
        Some(self.retained_matches_are_complete_but_for_custom_properties(node, matches))
    }

    fn retained_matches_are_complete_but_for_custom_properties(
        &self,
        node: StyleNodeID,
        matches: &[RetainedRuleMatch],
    ) -> bool {
        matches.iter().all(|rule_match| {
            let entry = &self.programs.get(rule_match.program).entries()[rule_match.entry as usize];
            self.match_is_complete_but_for_custom_properties(
                node,
                rule_match.rule,
                rule_match.tree_scope,
                entry.pseudo_element.is_some(),
            )
        })
    }

    /// Whether the winners the cascade publishes hold a match: its scope is one they are
    /// published for (`match_scope_is_complete_for`) and no container query gates it. A rule
    /// declares nothing past its longhand winners but custom properties.
    pub(super) fn match_is_complete_but_for_custom_properties(
        &self,
        node: StyleNodeID,
        rule: RuleID,
        tree_scope: TreeScopeID,
        pseudo: bool,
    ) -> bool {
        self.match_scope_is_complete_for(Some(node), rule, tree_scope)
            && self.container_gate_is_held(Some(node), rule, pseudo)
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

    pub(super) fn custom_declarations_read_attributes(&self, node: StyleNodeID, pseudo: Option<u8>) -> bool {
        self.cascaded_custom_declarations_of(node, pseudo)
            .is_some_and(|declarations| {
                declarations.iter().any(|(_, value)| {
                    matches!(
                        value.data(),
                        StyleValueData::Unresolved {
                            presence_attr: true,
                            ..
                        }
                    )
                })
            })
    }

    pub(super) fn custom_declarations_condition_usage(&self, node: StyleNodeID, pseudo: Option<u8>) -> u8 {
        self.cascaded_custom_declarations_of(node, pseudo)
            .map_or(0, |declarations| {
                declarations.iter().fold(0, |usage, (_, value)| {
                    usage
                        | match value.data() {
                            StyleValueData::Unresolved {
                                presence_if,
                                presence_inherit,
                                presence_dashed_function,
                                ..
                            } => {
                                u8::from(*presence_if)
                                    | (u8::from(*presence_inherit) << 1)
                                    | (u8::from(*presence_dashed_function) << 2)
                            }
                            _ => 0,
                        }
                })
            })
    }

    /// The element's own winning declaration, without importance inherited
    /// from an ancestor's flattened engine environment. Animation precedence
    /// reads this rather than the inherited store's entry metadata.
    pub(crate) fn cascaded_custom_property_importance(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        name_raw: usize,
    ) -> u8 {
        let Some(declarations) = self.cascaded_custom_declarations_of(node, pseudo) else {
            return 3;
        };
        declarations
            .into_iter()
            .find(|(declared, _)| {
                self.custom_property_environments
                    .name(declared.name)
                    .is_some_and(|name| name.raw.raw() == name_raw)
            })
            .map_or(0, |(declared, _)| if declared.important { 2 } else { 1 })
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

    pub(super) fn cascade_custom_declarations(
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
            // A gated rule declares for the node where its container conditions held when its
            // winners were published, as its longhands do.
            if !self.published_container_verdict_holds(node, rule, pseudo.is_some()) {
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
                                Some(node),
                                rule,
                                tree_scope,
                                specificity,
                                scope_proximity,
                                declared.important,
                            ),
                            self.cascade_stratum_of(Some(node), rule, tree_scope, declared.important),
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
        let inputs = self.document_style_computation_inputs;
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

    /// The name a cascaded custom declaration names, as its store entry keys it. A block's
    /// publication notes every custom property name it declares before the block is set
    /// (`collect_native_custom_declarations`), so a live declaration's name is always known.
    /// A replay notes names without their fly strings, and a name without one cannot key a
    /// store entry: `None` there, and the declaration declares nothing.
    fn declared_custom_property_name(&self, name: StyleAtomID) -> Option<&CustomPropertyName> {
        let noted = self.custom_property_environments.name(name);
        debug_assert!(
            noted.is_some(),
            "a custom declaration's name is noted at its publication"
        );
        noted.filter(|noted| noted.raw.raw() != 0)
    }

    /// The store behind an environment a node inherits, null for the empty one. A record that
    /// names an environment keeps its store alive (`retain_only` keeps what a record names), so
    /// a parent's is always held; one that is not inherits nothing.
    fn inherited_environment_store(&self, environment: u64) -> *const c_void {
        if environment == 0 {
            return std::ptr::null();
        }
        let store = self.custom_property_environments.store(environment);
        debug_assert!(store.is_some(), "an inherited environment keeps its store");
        store.unwrap_or(std::ptr::null())
    }

    /// The environment a child of an element holding `parent` inherits, where it is `parent` or
    /// one already built; `None` for a projection nobody built yet.
    pub(super) fn built_inheritable_custom_property_environment(
        &self,
        parent: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> Option<u64> {
        if parent == 0 || inputs.custom_property_registry.is_none() {
            return Some(parent);
        }
        let registry = unsafe {
            &*inputs
                .custom_property_registry
                .as_pointer()
                .cast::<CustomPropertyRegistry>()
        };
        if !registry.has_non_inheriting_registrations() {
            return Some(parent);
        }
        let key = Self::environment_inputs(parent, inputs.custom_property_registration_generation, &[]);
        self.custom_property_environments.memoized(&key)
    }

    pub(super) fn inheritable_custom_property_environment(
        &mut self,
        parent: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> u64 {
        if parent == 0 || inputs.custom_property_registry.is_none() {
            return parent;
        }
        let registry = unsafe {
            &*inputs
                .custom_property_registry
                .as_pointer()
                .cast::<CustomPropertyRegistry>()
        };
        if !registry.has_non_inheriting_registrations() {
            return parent;
        }
        let key = Self::environment_inputs(parent, inputs.custom_property_registration_generation, &[]);
        if let Some(identity) = self.custom_property_environments.memoized(&key) {
            return identity;
        }
        let source = self.inherited_environment_store(parent);
        if source.is_null() {
            return parent;
        }
        let store = unsafe { CustomPropertyStore::inheritable(source.cast(), registry) };
        let identity = if store == source {
            unsafe { Arc::decrement_strong_count(store.cast::<CustomPropertyStore>()) };
            parent
        } else if store.is_null() {
            0
        } else if let Some(identity) = self.custom_property_environments.identity_for_store(store) {
            unsafe { Arc::decrement_strong_count(store.cast::<CustomPropertyStore>()) };
            identity
        } else {
            // These names are inherited by the subject, including any important names.
            let flattened = unsafe { crate::css::custom_properties::rust_custom_property_store_flatten(store) };
            unsafe { Arc::decrement_strong_count(store.cast::<CustomPropertyStore>()) };
            let mut projection = unsafe { Arc::from_raw(flattened.cast::<CustomPropertyStore>()) };
            Arc::get_mut(&mut projection)
                .expect("fresh inherited projection")
                .declared_names
                .clear();
            unsafe {
                self.custom_property_environments
                    .adopt_engine_environment(Arc::into_raw(projection).cast(), 0)
            }
        };
        self.custom_property_environments.remember(key, identity, Vec::new());
        identity
    }

    /// The environment of a node the engine computes a record for: the one it inherits when its
    /// cascade declares no custom property, else what its declarations resolve to over that one.
    /// Refused when an input is missing: the registry, an interned name, or the store of the
    /// inherited environment.
    pub(super) fn engine_custom_property_environment(
        &mut self,
        node: StyleNodeID,
        parent_environment: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        registered: Option<RegisteredValueContext>,
        counters: &mut Counters,
    ) -> Drive<u64> {
        self.engine_custom_property_environment_of(node, None, parent_environment, inputs, registered, counters)
    }

    /// What `engine_custom_property_environment` says of the element, for one of its
    /// pseudo-elements over the element's own environment.
    pub(super) fn engine_custom_property_environment_of(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        parent_environment: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        registered: Option<RegisteredValueContext>,
        counters: &mut Counters,
    ) -> Drive<u64> {
        if !self.any_custom_property_is_declared() {
            return Ok(parent_environment);
        }
        let cascaded = Self::driven_custom_declarations(self.cascaded_custom_declarations_of(node, pseudo));
        self.engine_custom_property_environment_over(
            node,
            pseudo,
            cascaded,
            parent_environment,
            inputs,
            registered,
            counters,
        )
    }

    /// The custom declarations a drive resolves an environment from. A node the engine drives
    /// has the match answer its winners came from, and a published block carries a written value
    /// for each custom declaration, so the cascade always answers; one that does not declares
    /// nothing, and the node inherits its parent's environment.
    pub(super) fn driven_custom_declarations(
        cascaded: Option<Vec<(CustomDeclaration, RetainedStyleValueData)>>,
    ) -> Vec<(CustomDeclaration, RetainedStyleValueData)> {
        debug_assert!(cascaded.is_some(), "a driven node's custom declarations cascade");
        cascaded.unwrap_or_default()
    }

    /// What `engine_custom_property_environment_of` says of custom declarations cascaded for the
    /// node or pseudo-element by the caller.
    #[expect(
        clippy::too_many_arguments,
        reason = "the cascaded declarations and their independent resolution inputs travel together"
    )]
    pub(super) fn engine_custom_property_environment_over(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        cascaded: Vec<(CustomDeclaration, RetainedStyleValueData)>,
        parent_environment: u64,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        registered: Option<RegisteredValueContext>,
        counters: &mut Counters,
    ) -> Drive<u64> {
        if !self.any_custom_property_is_declared() {
            return Ok(parent_environment);
        }
        // Keep the unfiltered parent for an explicit inherit; ordinary inheritance drops
        // non-inheriting registrations before layering this element's declarations.
        let inheritance_environment = parent_environment;
        let parent_environment = self.inheritable_custom_property_environment(parent_environment, inputs);
        if cascaded.is_empty() {
            return Ok(parent_environment);
        }
        let reads_functions = cascaded.iter().any(|(_, value)| {
            matches!(
                value.data(),
                StyleValueData::Unresolved {
                    presence_dashed_function: true,
                    ..
                }
            )
        });
        let custom_functions = reads_functions.then(|| self.prepare_custom_functions(node, pseudo));
        let registry = inputs.custom_property_registry;
        let registry_ref = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
        let mut has_registered_declaration = false;
        if registry_ref.has_registrations() {
            for (declared, _) in &cascaded {
                let Some(name) = self.declared_custom_property_name(declared.name) else {
                    continue;
                };
                if registry_ref.registration_facts(&name.text).is_some() {
                    has_registered_declaration = true;
                    break;
                }
            }
        }
        // A registration with a real syntax computes its name's value against the element's own
        // font and viewport. A caller that brings no context resolves it against the record the
        // node holds, as a warm row does, or else provisionally against its parent's, as a first
        // record does before its drive settles the font.
        let registered = match registered {
            None if has_registered_declaration => Some(self.standing_registered_value_context(node, pseudo, inputs)),
            registered => registered,
        };
        let key = Self::environment_inputs(
            inheritance_environment,
            inputs.custom_property_registration_generation,
            &cascaded,
        );
        let parent_store = self.inherited_environment_store(parent_environment);
        let parent = unsafe { parent_store.cast::<CustomPropertyStore>().as_ref() };
        let mut values = Vec::with_capacity(cascaded.len());
        let mut reads_attributes = custom_functions
            .as_ref()
            .is_some_and(|functions| functions.reads_attributes);
        for (declared, value) in &cascaded {
            let Some(name) = self.declared_custom_property_name(declared.name) else {
                continue;
            };
            reads_attributes |= matches!(
                value.data(),
                StyleValueData::Unresolved {
                    presence_attr: true,
                    ..
                }
            );
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
            self.custom_property_environments
                .remember(key, parent_environment, written_values);
            return Ok(parent_environment);
        }
        // SAFETY: The parent store is live for as long as a record names its environment, and the
        // values are the program's interned values, live for the call.
        let cascaded_store = unsafe { CustomPropertyStore::cascaded_child(parent_store, values) };
        let mut random_sources = Vec::new();
        let random_values = crate::css::custom_properties::collect_registered_custom_property_random_sharings(
            unsafe { &*cascaded_store.cast::<CustomPropertyStore>() },
            registry_ref,
            &mut random_sources,
        );
        let random_bases = match self.random_base_values_for_sources(node, &random_sources) {
            Ok(random_bases) => random_bases,
            Err(unanswered) => {
                unsafe { Arc::decrement_strong_count(cascaded_store.cast::<CustomPropertyStore>()) };
                return Err(unanswered);
            }
        };
        // An environment C++ resolved for an element alike in its declarations is C++'s own
        // identity, and the host installs no record under one it does not recognise as the
        // engine's: handing it back settles a row the host then computes again. The memo is worth
        // only what it saves, so where it holds such an identity this resolves one of its own.
        // Random inputs can be element-scoped, so declarations alone cannot share their result.
        let reads_conditions = cascaded
            .iter()
            .any(|(_, value)| matches!(value.data(), StyleValueData::Unresolved { presence_if: true, .. }));
        let can_memoize = random_sources.is_empty()
            && !has_registered_declaration
            && !reads_attributes
            && !reads_conditions
            && !reads_functions;
        let memoized = can_memoize
            .then(|| self.custom_property_environments.memoized(&key))
            .flatten();
        let keeps_cpp_environment = memoized.is_some_and(|identity| {
            identity != parent_environment
                && identity & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT == 0
        });
        if let Some(identity) = memoized.filter(|_| !keeps_cpp_environment) {
            unsafe { Arc::decrement_strong_count(cascaded_store.cast::<CustomPropertyStore>()) };
            counters.bump(Counter::EngineCustomPropertyEnvironmentMemoHits);
            return Ok(identity);
        }
        let finalization_environment = crate::css::style_compute::FfiStyleComputationEnvironment {
            box_type_input: crate::css::style_compute::rust_box_type_transformation_input(
                0,
                crate::css::style_compute::FfiStyleAdjustmentTarget::Element,
                false,
                crate::css::display::FfiDisplay::block(),
            ),
            color_scheme_input: crate::css::style_compute::FfiEffectiveColorSchemeInput {
                preferred_color_scheme: 0,
                has_document_supported_schemes: false,
                document_supported_scheme_codes: std::ptr::null(),
                document_supported_scheme_count: 0,
            },
            is_th_element: false,
            has_new_font_size: false,
            has_tree_counting_context: false,
            sibling_count: 0,
            sibling_index: 0,
            random_base_values: random_bases.as_ptr(),
            random_base_value_count: random_bases.len(),
            document_base_url: std::ptr::null(),
            document_base_url_length: 0,
            style_sheet_resource_contexts: std::ptr::null(),
            style_sheet_resource_context_count: 0,
            device_pixels_per_css_pixel: inputs.device_pixels_per_css_pixel,
            initial_font_size_raw: inputs.initial_font_size_raw,
            default_font_size_raw: inputs.default_font_size_raw,
        };
        let mut random_function_index = 0_usize;
        let mut parse_context = registry_ref.parse_context(&mut random_function_index);
        parse_context.in_quirks_mode = inputs.in_quirks_mode;
        let length = registered
            .as_ref()
            .map_or(std::ptr::null(), |registered| &raw const registered.length);
        let attribute_element = self.substitution_attribute_element(node, pseudo);
        let substitution_attributes = if reads_attributes {
            self.facts
                .substitution_attributes(attribute_element)
                .iter()
                .map(
                    |(name, value)| crate::css::custom_properties::FfiSubstitutionAttribute {
                        name: FfiUtf16View {
                            ascii: std::ptr::null(),
                            utf16: name.as_ptr(),
                            length: name.len(),
                        },
                        value: FfiUtf16View {
                            ascii: std::ptr::null(),
                            utf16: value.as_ptr(),
                            length: value.len(),
                        },
                    },
                )
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let media_environment = self.document_media_snapshot.as_ffi();
        let mut resolution_context = engine_resolution_context(
            &parse_context,
            cascaded_store,
            self.custom_property_environments
                .store(inheritance_environment)
                .unwrap_or(std::ptr::null()),
            registry.as_pointer(),
            length,
            &substitution_attributes,
            !self.html_element_namespace.is_none()
                && self.facts.namespace_of(attribute_element) == self.html_element_namespace,
            &raw const media_environment,
        );
        if let Some(functions) = &custom_functions {
            resolution_context.custom_functions = functions.definitions.as_ptr();
            resolution_context.custom_function_count = functions.definitions.len();
            resolution_context.custom_function_scope_identity = functions.caller_scope;
            resolution_context.custom_function_visibilities = functions.visibilities.as_ptr();
            resolution_context.custom_function_visibility_count = functions.visibilities.len();
        }
        let drive = FfiCustomPropertyDriveInput {
            store: cascaded_store,
            resolved_parent_store: parent_store,
            reuse_resolved_parent_if_empty: !parent_store.is_null(),
            resolution_context: &raw const resolution_context,
            finalization_environment: &raw const finalization_environment,
            finalization_color_scheme: registered.as_ref().map_or(0, |registered| registered.color_scheme),
        };
        // SAFETY: Every pointer the drive reads is live for the call, and the finalizer replaces
        // each output with one transferred reference.
        let resolved = unsafe { drive_custom_property_resolution(&drive) };
        drop(random_values);
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
        if !keeps_cpp_environment && can_memoize {
            self.custom_property_environments
                .remember(key, identity, written_values);
        }
        Ok(identity)
    }

    /// What a written value with `var()` references substitutes to for a property under an
    /// environment, parsed as the property's value, memoized by the written value when it does not
    /// read the parent or attributes. A substitution that fails, or a result the property's grammar
    /// rejects, is the guaranteed-invalid value: the declaration is invalid at computed-value time.
    #[expect(
        clippy::too_many_arguments,
        reason = "the written value and its independent resolution inputs travel together"
    )]
    pub(super) fn substitute_written_value(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        environment: u64,
        property: u16,
        written: RetainedStyleValueData,
        inheritance_store: *const c_void,
        inputs: bridge::FfiDocumentStyleComputationInputs,
        counters: &mut Counters,
    ) -> RetainedStyleValueData {
        debug_assert!(
            value_is_engine_resolvable_substitution(written.data())
                || matches!(
                    written.data(),
                    StyleValueData::Unresolved {
                        presence_attr: true,
                        ..
                    }
                ),
            "only a value written with a substitution substitutes"
        );
        let reads_inheritance = matches!(
            written.data(),
            StyleValueData::Unresolved {
                presence_inherit: true,
                ..
            }
        );
        let reads_functions = matches!(
            written.data(),
            StyleValueData::Unresolved {
                presence_dashed_function: true,
                ..
            }
        );
        let reads_conditions = matches!(written.data(), StyleValueData::Unresolved { presence_if: true, .. });
        let functions = reads_functions.then(|| self.prepare_custom_functions(node, pseudo));
        let media_environment = self.document_media_snapshot.as_ffi();
        let resolution_inputs = OrdinarySubstitutionInputs {
            functions: functions.as_ref(),
            media_environment: &media_environment,
            style_query_length: self.document_media_snapshot.length.as_ref(),
        };
        // An `attr()` substitutes the element's own attributes, so the value is the element's and
        // takes no memo shared across the environment.
        let reads_attributes = matches!(
            written.data(),
            StyleValueData::Unresolved {
                presence_attr: true,
                ..
            }
        ) || functions.as_ref().is_some_and(|functions| functions.reads_attributes);
        let attribute_element = self.substitution_attribute_element(node, pseudo);
        let attributes = super::inputs::SubstitutionAttributeSnapshot {
            text: self.facts.substitution_attributes(attribute_element),
            names_are_ascii_case_insensitive: !self.html_element_namespace.is_none()
                && self.facts.namespace_of(attribute_element) == self.html_element_namespace,
        };
        let environments = &mut self.custom_property_environments;
        let memoizes = !reads_attributes && !reads_inheritance && !reads_functions && !reads_conditions;
        if memoizes && let Some(value) = environments.substitution(&written, property, environment) {
            counters.bump(Counter::EngineComputedRecordSubstitutionMemoHits);
            return value;
        }
        // A record names only environments whose stores the engine holds.
        let store = match environment {
            0 => std::ptr::null(),
            identity => environments.store(identity).unwrap_or_else(|| {
                debug_assert!(false, "a substitution environment without a store");
                std::ptr::null()
            }),
        };
        let value = substitute_written_value_against_store_with_attributes(
            store,
            inputs,
            property,
            &[],
            &written,
            &attributes,
            inheritance_store,
            Some(&resolution_inputs),
        );
        counters.bump(Counter::EngineComputedRecordSubstitutions);
        if memoizes {
            environments.remember_substitution(written, property, environment, value.clone_retained());
        }
        value
    }

    /// What a keyframe's written value substitutes to on the element being sampled, against the
    /// custom-property store the element holds now and the one it inherits from: what the host's
    /// `resolve_unresolved_style_value` makes of it. `root_custom_property_name` names the custom
    /// property the value is written for, and is empty for a longhand.
    ///
    /// Like a cascaded declaration, a value that does not substitute or parse is guaranteed-invalid
    /// rather than declined.
    #[expect(
        clippy::too_many_arguments,
        reason = "the written value and its independent resolution inputs travel together"
    )]
    pub(crate) fn substitute_keyframe_value(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        store: *const c_void,
        inheritance_store: *const c_void,
        property: u16,
        root_custom_property_name: &[u16],
        written: &RetainedStyleValueData,
    ) -> RetainedStyleValueData {
        let inputs = self.document_style_computation_inputs;
        let functions = match written.data() {
            StyleValueData::Unresolved {
                presence_dashed_function: true,
                ..
            } => Some(self.prepare_custom_functions(node, pseudo)),
            _ => None,
        };
        let media_environment = self.document_media_snapshot.as_ffi();
        let resolution_inputs = OrdinarySubstitutionInputs {
            functions: functions.as_ref(),
            media_environment: &media_environment,
            style_query_length: self.document_media_snapshot.length.as_ref(),
        };
        let attribute_element = self.substitution_attribute_element(node, pseudo);
        let attributes = super::inputs::SubstitutionAttributeSnapshot {
            text: self.facts.substitution_attributes(attribute_element),
            names_are_ascii_case_insensitive: !self.html_element_namespace.is_none()
                && self.facts.namespace_of(attribute_element) == self.html_element_namespace,
        };
        substitute_written_value_against_store_with_attributes(
            store,
            inputs,
            property,
            root_custom_property_name,
            written,
            &attributes,
            inheritance_store,
            Some(&resolution_inputs),
        )
    }
}

struct OrdinarySubstitutionInputs<'a> {
    functions: Option<&'a PreparedCustomFunctions>,
    media_environment: &'a crate::css::parser::query_parser::FfiMediaEnvironment,
    style_query_length: Option<&'a crate::css::style_compute::FfiLengthResolutionContext>,
}

/// What a written value with `var()` references substitutes to for a property against a resolved
/// custom-property store, parsed as the property's value. Memo-free, for the compositor, which
/// holds the store but not the element: `None` when the value reads the element's attributes,
/// its parent's environment, custom functions or conditions, which the main thread's sampling
/// substitutes instead.
pub(crate) fn substitute_written_value_against_store(
    store: *const c_void,
    inputs: bridge::FfiDocumentStyleComputationInputs,
    property: u16,
    written: &RetainedStyleValueData,
) -> Option<RetainedStyleValueData> {
    if !custom_property_value_is_engine_resolvable(written.data()) {
        return None;
    }
    Some(substitute_written_value_against_store_with_attributes(
        store,
        inputs,
        property,
        &[],
        written,
        &super::inputs::SubstitutionAttributeSnapshot::default(),
        std::ptr::null(),
        None,
    ))
}

/// The same substitution with the element's attributes and published resolution inputs. A
/// substitution that fails, or a source the property's grammar rejects, is guaranteed-invalid.
#[expect(
    clippy::too_many_arguments,
    reason = "the ordinary and animation callers provide distinct resolution inputs"
)]
fn substitute_written_value_against_store_with_attributes(
    store: *const c_void,
    inputs: bridge::FfiDocumentStyleComputationInputs,
    property: u16,
    root_custom_property_name: &[u16],
    written: &RetainedStyleValueData,
    attributes: &super::inputs::SubstitutionAttributeSnapshot<'_>,
    inheritance_store: *const c_void,
    resolution_inputs: Option<&OrdinarySubstitutionInputs<'_>>,
) -> RetainedStyleValueData {
    let guaranteed_invalid = || RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid);
    let substitution_attributes = attributes
        .text
        .iter()
        .map(
            |&(name, value)| crate::css::custom_properties::FfiSubstitutionAttribute {
                name: FfiUtf16View {
                    ascii: std::ptr::null(),
                    utf16: name.as_ptr(),
                    length: name.len(),
                },
                value: FfiUtf16View {
                    ascii: std::ptr::null(),
                    utf16: value.as_ptr(),
                    length: value.len(),
                },
            },
        )
        .collect::<Vec<_>>();
    // The document publishes its registry with every transaction's inputs.
    let registry = inputs.custom_property_registry;
    let registry_ref = unsafe { &*registry.as_pointer().cast::<CustomPropertyRegistry>() };
    let mut random_function_index = 0_usize;
    let mut parse_context = registry_ref.parse_context(&mut random_function_index);
    parse_context.in_quirks_mode = inputs.in_quirks_mode;
    let functions = resolution_inputs.and_then(|inputs| inputs.functions);
    let Some(mut resolution_environment) = (unsafe {
        prepare_var_resolution_environment(
            substitution_attributes.as_ptr(),
            substitution_attributes.len(),
            functions.map_or(std::ptr::null(), |functions| functions.definitions.as_ptr()),
            functions.map_or(0, |functions| functions.definitions.len()),
            functions.map_or(0, |functions| functions.caller_scope),
            functions.map_or(std::ptr::null(), |functions| functions.visibilities.as_ptr()),
            functions.map_or(0, |functions| functions.visibilities.len()),
        )
    }) else {
        debug_assert!(false, "custom function definitions that do not describe a registry");
        return guaranteed_invalid();
    };
    // SAFETY: The store is live while a record names its environment, and the written value
    // is retained by the declaration that carries it.
    let resolution = unsafe {
        crate::css::custom_properties::resolve_vars(
            store,
            inheritance_store,
            registry.as_pointer(),
            Some(&parse_context),
            resolution_inputs.map(|inputs| inputs.media_environment),
            property,
            FfiUtf16View {
                ascii: std::ptr::null(),
                utf16: root_custom_property_name.as_ptr(),
                length: root_custom_property_name.len(),
            },
            written.pointer().cast(),
            &mut resolution_environment,
            attributes.names_are_ascii_case_insensitive,
            resolution_inputs
                .and_then(|inputs| inputs.style_query_length)
                .map_or(std::ptr::null(), std::ptr::from_ref),
            std::ptr::null_mut(),
            None,
        )
    };
    // The substituted source parses without callbacks first, then with the parse context's. A
    // substitution the resolver cannot make, or a source no grammar accepts, leaves the
    // declaration invalid at computed-value time.
    match resolution {
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
                ParseOutcome::Invalid | ParseOutcome::NotHandled => guaranteed_invalid(),
            }
        }
        NativeVarResolution::Invalid | NativeVarResolution::NotHandled => guaranteed_invalid(),
    }
}
