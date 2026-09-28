/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::{CompilationContext, NativeRuleType, NativeStyleSheet};
use crate::css::container_conditions::ContainerConditionsData;
use crate::css::declaration_block::DeclarationBlockData;
use crate::css::rule::read::RuleRef;
use crate::css::selector::SelectorList;
use crate::css::selector_operations::{
    absolutize_selector_list, adapt_scope_end_selector_list, scope_root_selector_list,
};
use crate::css::selector_parser::{RustParsedSelectorList, StyleNestingParent};
use crate::css::style::bridge::{
    BoundScopeChain, operations, publish_rule_declarations, publish_style_rule, publish_style_rule_selectors,
};
use crate::css::style::compiler::NamespaceScope;
use std::rc::Rc;
use std::sync::Arc;

/// What the host hears of a rule a walk compiled: whether the engine publishes it, and whether its declarations
/// declare transitions.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct NativeCompilationResult {
    pub published: bool,
    pub declares_transitions: bool,
}

/// Where compiled rules go: the document's style engine, the sheet (its id plus one), and the identity of the rule
/// they go before, or zero for the end of the sheet. Neither the parser nor the native stylesheet graph retains document
/// engines, interners, or host callbacks.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeStylePublication {
    pub engine: crate::css::style::StyleEngineInputHandle,
    pub sheet: u32,
    pub before: u64,
}

// Binding belongs to this document-thread traversal, not to the shared rule graph.
#[derive(Clone)]
pub(super) struct SelectorInputs {
    parent_kind: StyleNestingParent,
    immediate_parent_kind: StyleNestingParent,
    parents: Option<Rc<RustParsedSelectorList>>,
    scope: BoundScopeChain,
}

impl Default for SelectorInputs {
    fn default() -> Self {
        Self {
            parent_kind: StyleNestingParent::None,
            immediate_parent_kind: StyleNestingParent::None,
            parents: None,
            scope: BoundScopeChain::default(),
        }
    }
}

impl SelectorInputs {
    unsafe fn bind(
        &self,
        source: &RustParsedSelectorList,
        parent_kind: StyleNestingParent,
    ) -> Rc<RustParsedSelectorList> {
        let empty: crate::css::selector::SelectorList = Box::new([]);
        let parents = if parent_kind == StyleNestingParent::Style {
            self.parents.as_ref().map_or(&empty, |parents| &parents.selectors)
        } else {
            &empty
        };
        Rc::new(RustParsedSelectorList {
            selectors: absolutize_selector_list(&source.selectors, parent_kind, parents)
                .unwrap_or_else(|| source.selectors.clone()),
        })
    }

    pub(super) unsafe fn matching_selectors(&self, rule: RuleRef<'_>) -> Option<Rc<RustParsedSelectorList>> {
        match rule.rule_type() {
            NativeRuleType::Style => Some(unsafe { self.bind(&rule.selectors().unwrap(), self.parent_kind) }),
            NativeRuleType::NestedDeclarations => {
                if let Some(parents) = &self.parents {
                    return Some(parents.clone());
                }
                assert_eq!(self.parent_kind, StyleNestingParent::Scope);
                Some(Rc::new(RustParsedSelectorList {
                    selectors: scope_root_selector_list(),
                }))
            }
            _ => None,
        }
    }

    pub(super) unsafe fn within(
        &self,
        rule: RuleRef<'_>,
        matching: Option<Rc<RustParsedSelectorList>>,
        implicit_root: u32,
    ) -> Self {
        let mut nested = self.clone();
        nested.immediate_parent_kind = StyleNestingParent::None;
        if rule.rule_type() == NativeRuleType::Style {
            nested.parents = matching.or_else(|| unsafe { self.matching_selectors(rule) });
            nested.parent_kind = StyleNestingParent::Style;
            nested.immediate_parent_kind = StyleNestingParent::Style;
        }
        if let Some(scope) = rule.scope() {
            let parent_kind = if rule.rule_type() == NativeRuleType::Import {
                StyleNestingParent::None
            } else {
                self.immediate_parent_kind
            };
            let start = scope
                .start
                .as_ref()
                .map(|start| unsafe { self.bind(start, parent_kind) });
            let end = scope.end.as_ref().map(|end| RustParsedSelectorList {
                selectors: adapt_scope_end_selector_list(&end.selectors),
            });
            nested.scope.push(start.as_deref(), end.as_ref(), implicit_root);
        }
        match rule.rule_type() {
            NativeRuleType::Scope => {
                nested.parent_kind = StyleNestingParent::Scope;
                nested.immediate_parent_kind = StyleNestingParent::Scope;
                nested.parents = None;
            }
            NativeRuleType::Import => {
                nested.parent_kind = StyleNestingParent::None;
                nested.parents = None;
            }
            _ => {}
        }
        nested
    }
}

/// Where the main thread's walk plans the rules it compiles into the document's style engine: the sheet, the rule
/// they go before, and what it planned so far, which the render owner publishes as it applies the change that carries
/// it. What the host hears of each rule, the walk knows without the engine.
pub(super) struct Planner {
    pub(super) sheet: u32,
    /// The identity of the rule the compiled rules go before, or zero for the end of the sheet.
    pub(super) before: u64,
    pub(super) plan: Vec<Planned>,
    /// The namespaces of each sheet the walk compiled rules of, which its rules share.
    namespaces: Vec<(*const NativeStyleSheet, Arc<SheetNamespaces>)>,
}

/// A step of a [`Planner`]'s plan.
pub(crate) enum Planned {
    Compile(CompiledRule),
    ReplaceSelectors(ReplacedSelectors),
}

/// The namespaces a sheet declares, as the text the engine interns: the default one, and each prefix with the one it
/// names.
pub(crate) struct SheetNamespaces {
    default: Option<Text>,
    by_prefix: Box<[(Text, Text)]>,
}

type Text = Box<[u16]>;

impl SheetNamespaces {
    fn of(sheet: &NativeStyleSheet) -> Self {
        let mut default = None;
        let mut by_prefix = Vec::new();
        sheet.rules().for_each_namespace(|namespace| {
            let uri = namespace.uri.units().into();
            match namespace.prefix.units() {
                [] => default = Some(uri),
                prefix => by_prefix.push((prefix.into(), uri)),
            }
        });
        Self {
            default,
            by_prefix: by_prefix.into(),
        }
    }

    fn intern(&self, engine: &mut crate::css::style::StyleEngine) -> NamespaceScope {
        let mut intern = |text: &[u16]| match text {
            [] => crate::css::style::index::StyleAtomID::NONE,
            text => crate::css::style::bridge::intern_native_text(engine, text),
        };
        NamespaceScope {
            default: self.default.as_deref().map(&mut intern),
            by_prefix: self
                .by_prefix
                .iter()
                .map(|(prefix, uri)| (intern(prefix), intern(uri)))
                .collect(),
        }
    }
}

/// A rule the main thread's walk compiled, with all the engine publishes of it.
pub(crate) struct CompiledRule {
    identity: u64,
    /// The identity of the rule it goes before, or zero for the end of the sheet.
    before: u64,
    kind: CompiledRuleKind,
    declarations: Option<Arc<DeclarationBlockData>>,
    source_identity: u64,
    conditions_hold: bool,
    /// Whether it cascades in the layer `layer_name` names.
    in_a_layer: bool,
    layer_name: Box<[u16]>,
    containers: Box<[Arc<ContainerConditionsData>]>,
}

enum CompiledRuleKind {
    Style {
        selectors: SelectorList,
        namespaces: Arc<SheetNamespaces>,
        scope: BoundScopeChain,
        gated_by_container_query: bool,
    },
    FontFeatureValues,
    CounterStyle,
    Function,
    Property(Box<[u16]>),
    Keyframes(Box<[u16]>),
}

/// The selectors a style rule the engine holds now has.
pub(crate) struct ReplacedSelectors {
    identity: u64,
    selectors: SelectorList,
    namespaces: Arc<SheetNamespaces>,
    scope: BoundScopeChain,
}

impl Planner {
    pub(super) fn new(sheet: u32, before: u64) -> Self {
        Self {
            sheet,
            before,
            plan: Vec::new(),
            namespaces: Vec::new(),
        }
    }

    /// Whether the plan compiles the rule of `identity`.
    pub(super) fn compiles(&self, identity: u64) -> bool {
        self.plan
            .iter()
            .any(|planned| matches!(planned, Planned::Compile(rule) if rule.identity == identity))
    }

    fn namespaces(&mut self, sheet: &NativeStyleSheet) -> Arc<SheetNamespaces> {
        let key = std::ptr::from_ref(sheet);
        if let Some((_, namespaces)) = self.namespaces.iter().find(|(sheet, _)| *sheet == key) {
            return namespaces.clone();
        }
        let namespaces = Arc::new(SheetNamespaces::of(sheet));
        self.namespaces.push((key, namespaces.clone()));
        namespaces
    }

    pub(super) fn replace_selectors(
        &mut self,
        rule: RuleRef<'_>,
        source: &NativeStyleSheet,
        context: &CompilationContext,
        selectors: &RustParsedSelectorList,
    ) {
        let namespaces = self.namespaces(source);
        self.plan.push(Planned::ReplaceSelectors(ReplacedSelectors {
            identity: rule.identity(),
            selectors: selectors.selectors.clone(),
            namespaces,
            scope: context.selectors.scope.clone(),
        }));
    }

    /// Plans `rule`, and answers what the host hears of it.
    pub(super) fn compile(
        &mut self,
        rule: RuleRef<'_>,
        source: &NativeStyleSheet,
        context: &CompilationContext,
        selectors: Option<&RustParsedSelectorList>,
    ) -> NativeCompilationResult {
        let rule_type = rule.rule_type();
        let declarations = rule.cascade_declarations();
        let kind = match rule_type {
            NativeRuleType::Style | NativeRuleType::NestedDeclarations => {
                let selectors = selectors.unwrap();
                if self.sheet == 0 || selectors.selectors.is_empty() {
                    return NativeCompilationResult::default();
                }
                CompiledRuleKind::Style {
                    selectors: selectors.selectors.clone(),
                    namespaces: self.namespaces(source),
                    scope: context.selectors.scope.clone(),
                    gated_by_container_query: context.gated_by_container_query,
                }
            }
            NativeRuleType::FontFeatureValues => CompiledRuleKind::FontFeatureValues,
            NativeRuleType::CounterStyle => CompiledRuleKind::CounterStyle,
            NativeRuleType::Function => CompiledRuleKind::Function,
            NativeRuleType::Property => CompiledRuleKind::Property(rule.definition_name().unwrap().units().into()),
            NativeRuleType::Keyframes => CompiledRuleKind::Keyframes(rule.definition_name().unwrap().units().into()),
            _ => return NativeCompilationResult::default(),
        };
        let declares_transitions = matches!(kind, CompiledRuleKind::Style { .. })
            && declarations.as_ref().is_some_and(|declarations| {
                declarations.properties.iter().any(|declaration| {
                    crate::css::property_metadata::property_defines_a_css_transition(declaration.property_id)
                })
            });
        self.plan.push(Planned::Compile(CompiledRule {
            identity: rule.identity(),
            before: self.before,
            kind,
            declarations,
            source_identity: source.identity(),
            conditions_hold: context.conditions_hold,
            in_a_layer: context.in_a_layer
                && matches!(
                    rule_type,
                    NativeRuleType::Style | NativeRuleType::NestedDeclarations | NativeRuleType::CounterStyle
                ),
            layer_name: context.layer_name.as_slice().into(),
            containers: context
                .containers
                .iter()
                .map(|&container| {
                    // SAFETY: The walk's containers point into the Arc allocations the rules hold.
                    unsafe {
                        Arc::increment_strong_count(container);
                        Arc::from_raw(container)
                    }
                })
                .collect(),
        }));
        NativeCompilationResult {
            published: true,
            declares_transitions,
        }
    }
}

impl Planned {
    /// The identity of the rule the step compiles, if it compiles one.
    pub(crate) fn compiled_identity(&self) -> Option<u64> {
        match self {
            Self::Compile(rule) => Some(rule.identity),
            Self::ReplaceSelectors(_) => None,
        }
    }

    /// Publishes the step into `engine`, the one the owner answers from, into the sheet `sheet`.
    pub(crate) fn publish(self, engine: &mut crate::css::style::StyleEngine, sheet: u32) {
        match self {
            Self::Compile(rule) => rule.publish(engine, sheet),
            Self::ReplaceSelectors(replaced) => {
                let Some(id) = engine.native_rule_id(replaced.identity) else {
                    return;
                };
                let namespaces = replaced.namespaces.intern(engine);
                let compiled: Vec<_> = replaced.selectors.iter().map(AsRef::as_ref).collect();
                publish_style_rule_selectors(engine, id.0 + 1, &compiled, namespaces, &replaced.scope);
            }
        }
    }
}

impl CompiledRule {
    fn publish(self, engine: &mut crate::css::style::StyleEngine, sheet: u32) {
        // The rule it goes before is the one the engine holds by then, as the changes before this one left it.
        let before = match self.before {
            0 => 0,
            identity => engine.native_rule_id(identity).map_or(0, |id| id.0 + 1),
        };
        // Reuse the engine's recorded publication operations so recording and replay see the
        // same semantic inputs as incremental CSSOM edits.
        let rule_id = match self.kind {
            CompiledRuleKind::Style {
                selectors,
                namespaces,
                scope,
                gated_by_container_query,
            } => {
                let namespaces = namespaces.intern(engine);
                let compiled: Vec<_> = selectors.iter().map(AsRef::as_ref).collect();
                let id = publish_style_rule(engine, sheet, before, &compiled, namespaces, &scope);
                publish_rule_declarations(engine, id, self.declarations.as_deref().unwrap());
                if gated_by_container_query {
                    operations::set_rule_gated_by_container_query(engine, id);
                }
                id
            }
            CompiledRuleKind::FontFeatureValues => operations::add_font_feature_values_rule(engine, sheet, before),
            CompiledRuleKind::CounterStyle => operations::add_counter_style_rule(engine, sheet, before),
            CompiledRuleKind::Function => operations::add_function_rule(engine, sheet, before),
            CompiledRuleKind::Property(name) => {
                let name = crate::css::style::bridge::intern_native_text(engine, &name).0;
                operations::add_property_rule(engine, sheet, before, name)
            }
            CompiledRuleKind::Keyframes(name) => {
                let name = crate::css::style::bridge::intern_native_text(engine, &name).0;
                operations::add_keyframes_rule(engine, sheet, before, name)
            }
        };
        if rule_id == 0 {
            return;
        }
        if !self.conditions_hold {
            operations::set_rule_conditions_hold(engine, rule_id, false);
        }
        if self.in_a_layer {
            let layer = crate::css::style::bridge::intern_native_text(engine, &self.layer_name).0;
            operations::set_rule_in_a_layer(engine, rule_id);
            operations::set_rule_layer(engine, rule_id, layer);
        }
        let containers: Vec<_> = self.containers.iter().map(Arc::as_ptr).collect();
        // SAFETY: The containers are live Arc allocations, which the rule holds on.
        unsafe {
            engine.register_native_rule(
                crate::css::style::program::RuleID(rule_id - 1),
                self.identity,
                self.declarations,
                self.source_identity,
                &self.layer_name,
                &containers,
            );
        }
    }
}

/// The rules a walk of one sheet compiled, and the selectors it replaced, in the order the walk planned them, which
/// the main thread sends the render owner as one change.
pub(crate) struct CompiledRules {
    sheet: u32,
    plan: Vec<Planned>,
}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<CompiledRules>();
};

impl CompiledRules {
    /// The identities of the rules the change compiles.
    pub(crate) fn compiled_identities(&self) -> impl Iterator<Item = u64> + '_ {
        self.plan.iter().filter_map(Planned::compiled_identity)
    }

    /// Visits the ASCII-lowercase local name of every attribute the rules' selectors and scopes test the value of by
    /// its text, as [`crate::css::selector::CompiledSelector::visit_attribute_value_text_names`] does.
    pub(crate) fn visit_attribute_value_text_names(
        &self,
        visit: &mut impl FnMut(&crate::css::retained_fly_string::RetainedUtf16FlyString),
    ) {
        for planned in &self.plan {
            let (selectors, scope) = match planned {
                Planned::Compile(CompiledRule {
                    kind: CompiledRuleKind::Style { selectors, scope, .. },
                    ..
                }) => (selectors, scope),
                Planned::ReplaceSelectors(replaced) => (&replaced.selectors, &replaced.scope),
                Planned::Compile(_) => continue,
            };
            for selector in selectors.iter().map(AsRef::as_ref).chain(scope.selectors()) {
                selector.visit_attribute_value_text_names(visit);
            }
        }
    }

    /// Publishes the rules into `engine`, on the owner.
    pub(crate) fn publish(self, engine: &mut crate::css::style::StyleEngine) {
        for planned in self.plan {
            planned.publish(engine, self.sheet);
        }
    }
}

impl Planner {
    /// What the planner planned, as the change the main thread sends, where it planned anything.
    pub(super) fn into_compiled_rules(self) -> Option<CompiledRules> {
        (!self.plan.is_empty()).then_some(CompiledRules {
            sheet: self.sheet,
            plan: self.plan,
        })
    }
}
