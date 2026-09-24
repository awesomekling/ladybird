/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Rust-owned custom-property data with the same structurally shared parent shape as the C++
//! `CustomPropertyData` shell.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use ak::ScopeGuard;

use crate::css::css_tokenizer::OwnedToken;
use crate::css::css_tokenizer::OwnedTokenKind;
use crate::css::css_tokenizer::tokenize_owned;
use crate::css::ffi_support::FfiUtf16View;
use crate::css::function_signature::FunctionSignature;
use crate::css::parser::query_parser::{
    FfiMediaEnvironment, MatchResult, parse_and_evaluate_media_if_condition, parse_and_evaluate_supports_if_condition,
};
use crate::css::parser::syntax::{SyntaxNode, SyntaxType, clone_syntax_handle, parse_syntax, parse_with_syntax};
use crate::css::parser::value_parser::{FfiValueParsingContext, FfiValueParsingContextKind, ParseContext};
use crate::css::retained_fly_string::RetainedUtf16FlyString;
use crate::css::style_value::RetainedStyleValueData;
use crate::css::style_value::StyleValueData;

include!(concat!(env!("OUT_DIR"), "/environment_variables_generated.rs"));

// NB: Mirrors the lookup that Meta/Generators/generate_libweb_css_environment_variables.py
//     emitted before this series removed that generator.
pub(crate) fn environment_variable_is_known(name: &[u16]) -> bool {
    ENVIRONMENT_VARIABLES
        .iter()
        .any(|(known_name, _, _)| name.eq_ignore_ascii_case(known_name))
}
// Keep these structural-sharing limits aligned with CustomPropertyData.cpp.
const MAX_ANCESTOR_COUNT: u8 = 32;
const ABSORB_THRESHOLD: usize = 8;

trait Utf16SliceExt {
    fn eq_ignore_ascii_case(&self, expected: &str) -> bool;
    fn eq_ignore_ascii_case_utf16(&self, expected: &[u16]) -> bool;
    fn starts_with_ascii(&self, expected: &str) -> bool;
}

impl Utf16SliceExt for [u16] {
    fn eq_ignore_ascii_case(&self, expected: &str) -> bool {
        self.len() == expected.len()
            && self
                .iter()
                .zip(expected.bytes())
                .all(|(&left, right)| left <= 0x7f && (left as u8).eq_ignore_ascii_case(&right))
    }

    fn starts_with_ascii(&self, expected: &str) -> bool {
        self.len() >= expected.len()
            && self[..expected.len()]
                .iter()
                .zip(expected.bytes())
                .all(|(&left, right)| left == u16::from(right))
    }

    fn eq_ignore_ascii_case_utf16(&self, expected: &[u16]) -> bool {
        self.len() == expected.len()
            && self.iter().zip(expected).all(|(&left, &right)| {
                left == right
                    || u8::try_from(left)
                        .ok()
                        .zip(u8::try_from(right).ok())
                        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(&right))
            })
    }
}

#[repr(C)]
pub struct FfiCustomPropertyStoreEntry {
    pub name_raw: usize,
    pub name: FfiUtf16View,
    pub important: bool,
    pub data: *const c_void,
}

#[derive(Clone)]
pub(crate) struct CustomPropertyEntry {
    _name: RetainedUtf16FlyString,
    pub(crate) name: Arc<[u16]>,
    pub(crate) value: RetainedStyleValueData,
    pub(crate) important: bool,
}

pub struct CustomPropertyStore {
    pub(crate) own_values: HashMap<usize, CustomPropertyEntry>,
    pub(crate) declared_names: Vec<usize>,
    own_names: HashMap<Arc<[u16]>, usize>,
    parent: Option<Arc<CustomPropertyStore>>,
    inheritance_parent: Option<Arc<CustomPropertyStore>>,
    ancestor_count: u8,
}

// SAFETY: Store nodes and their entries are immutable after construction. Style workers only
// borrow the graph while resolving substitution text; the C++-owned raw Arc reference remains
// alive until every worker in the blocking batch has joined, so destruction stays on the main
// thread. The retained style values and names are likewise only read by workers.
unsafe impl Send for CustomPropertyStore {}
unsafe impl Sync for CustomPropertyStore {}

#[derive(Clone)]
pub struct CustomPropertyRegistry {
    registrations: HashMap<Vec<u16>, RegisteredCustomProperty>,
    document_url: Vec<u8>,
    document_base_url: Vec<u8>,
}

#[derive(Clone)]
struct RegisteredCustomProperty {
    syntax: SyntaxNode,
    inherits: bool,
    initial_source: Option<Vec<u16>>,
    /// What `compute_registered_custom_property_initial_value` settled for this registration,
    /// published with it. The computation resolves lengths against the *document* - the initial
    /// font and the viewport, not the element - so it is a fact about the registration rather than
    /// about whoever reads it, and the host already memoizes it on the registration and drops the
    /// memo when the viewport moves. Absent only where the registry was filled without one.
    computed_initial: Option<RetainedStyleValueData>,
}

/// What a `@property` registration decides about a name, for a caller that answers for it without
/// the host: whether it inherits, whether its specified value has to be computed against the
/// registered syntax at all, and what the registration's initial value computed to. The initial
/// value is absent only where the registry was filled without the host's published one.
pub(crate) struct RegistrationFacts {
    pub(crate) inherits: bool,
    pub(crate) computes_a_specified_value: bool,
    pub(crate) initial_value: Option<RetainedStyleValueData>,
}

type CustomFunctionIdentity = u64;

#[derive(Clone)]
struct CustomFunctionDefinition {
    identity: CustomFunctionIdentity,
    scope_identity: usize,
    signature: Arc<FunctionSignature>,
    parameter_defaults: Vec<Option<Vec<OwnedToken>>>,
    declarations: Vec<(Vec<u16>, Vec<OwnedToken>, bool)>,
}

struct CustomFunctionRegistry {
    caller_scope_identity: usize,
    definitions: Vec<CustomFunctionDefinition>,
    visible_definitions: HashMap<(usize, Vec<u16>), CustomFunctionIdentity>,
}

#[derive(Clone)]
struct FunctionLocalRegistration {
    syntax: SyntaxNode,
    initial_tokens: Option<Vec<OwnedToken>>,
    is_result: bool,
}

#[derive(Clone)]
struct FunctionLocalValue {
    tokens: Vec<OwnedToken>,
    includes_substitution: bool,
}

struct FunctionLocalScope {
    function_identity: CustomFunctionIdentity,
    resolved_value_cache: Rc<RefCell<HashMap<Vec<u16>, Vec<OwnedToken>>>>,
    values: HashMap<Vec<u16>, FunctionLocalValue>,
    registrations: Rc<HashMap<Vec<u16>, FunctionLocalRegistration>>,
}

#[repr(C)]
pub struct FfiCustomPropertyRegistration {
    pub name: FfiUtf16View,
    pub syntax: *const c_void,
    pub inherits: bool,
    pub has_initial_value: bool,
    pub initial_value: FfiUtf16View,
    /// The computed initial value the host derived from `initial_value` against the document,
    /// borrowed for the call and retained by the registry. Null where the host has none.
    pub computed_initial_value: *const c_void,
}

#[repr(C)]
pub struct FfiCustomPropertyRegistryContext {
    pub document_url: *const u8,
    pub document_url_length: usize,
    pub document_base_url: *const u8,
    pub document_base_url_length: usize,
}

#[repr(C)]
pub struct FfiSubstitutionAttribute {
    pub name: FfiUtf16View,
    pub value: FfiUtf16View,
}

#[repr(C)]
pub struct FfiSubstitutionFunctionDeclaration {
    pub name: FfiUtf16View,
    pub data: *const c_void,
}

#[repr(C)]
pub struct FfiSubstitutionFunctionDefinition {
    pub identity: u64,
    pub scope_identity: usize,
    pub signature: *const c_void,
    pub declarations: *const FfiSubstitutionFunctionDeclaration,
    pub declaration_count: usize,
}

#[repr(C)]
pub struct FfiSubstitutionFunctionVisibility {
    pub caller_scope_identity: usize,
    pub function_identity: u64,
}

impl CustomPropertyRegistry {
    /// Whether any custom property is registered; an unregistered name resolves without a syntax.
    pub(crate) fn has_registrations(&self) -> bool {
        !self.registrations.is_empty()
    }

    /// Whether descendants need a filtered projection of their parent's environment.
    pub(crate) fn has_non_inheriting_registrations(&self) -> bool {
        self.registrations.values().any(|registration| !registration.inherits)
    }

    /// What a registration says about a name, for a caller that has to answer for it without the
    /// host: `None` where the name is not registered at all.
    pub(crate) fn registration_facts(&self, name: &[u16]) -> Option<RegistrationFacts> {
        let registration = self.registrations.get(name)?;
        Some(RegistrationFacts {
            inherits: registration.inherits,
            computes_a_specified_value: !matches!(registration.syntax, SyntaxNode::Universal),
            initial_value: registration.computed_initial.clone(),
        })
    }

    pub(crate) fn parse_context(&self, random_function_index: &mut usize) -> ParseContext {
        ParseContext {
            in_quirks_mode: false,
            is_svg_presentation_attribute: false,
            is_substituted_value: false,
            contains_attr_tainted_values: false,
            is_ua_style_sheet: false,
            value_contexts: std::ptr::null(),
            value_context_count: 0,
            declared_namespaces: std::ptr::null(),
            document_url: self.document_url.as_ptr(),
            document_url_length: self.document_url.len(),
            document_base_url: self.document_base_url.as_ptr(),
            document_base_url_length: self.document_base_url.len(),
            length_resolution_context: std::ptr::null(),
            random_function_index,
        }
    }
}

pub(crate) fn collect_registered_custom_property_random_sharings(
    store: &CustomPropertyStore,
    registry: &CustomPropertyRegistry,
    sharings: &mut Vec<*const StyleValueData>,
) -> Vec<RetainedStyleValueData> {
    let mut parsed_values = Vec::new();
    for entry in store.own_values.values() {
        let initial_count = sharings.len();
        crate::css::style_compute::collect_unfixed_random_sharings_in_value(entry.value.data(), sharings);
        if sharings.len() != initial_count {
            continue;
        }
        let Some(registration) = registry.registrations.get(entry.name.as_ref()) else {
            continue;
        };
        if matches!(registration.syntax, SyntaxNode::Universal) {
            continue;
        }
        let Some(source) = crate::css::serialize::serialize_resolved_style_value_to_utf16(entry.value.data()) else {
            continue;
        };
        let mut random_function_index = 0;
        let value_context = FfiValueParsingContext {
            kind: FfiValueParsingContextKind::Property,
            value: crate::css::property_metadata::property_id::CUSTOM,
            secondary_value: 0,
            name: Default::default(),
        };
        let mut parse_context = registry.parse_context(&mut random_function_index);
        parse_context.value_contexts = &raw const value_context;
        parse_context.value_context_count = 1;
        let Some(parsed) = parse_with_syntax(&parse_context, &source, &registration.syntax) else {
            continue;
        };
        let parsed = RetainedStyleValueData::from_owned(parsed);
        crate::css::style_compute::collect_unfixed_random_sharings_in_value(parsed.data(), sharings);
        parsed_values.push(parsed);
    }
    parsed_values
}

fn registered_initial_value(
    registry: &CustomPropertyRegistry,
    registration: &RegisteredCustomProperty,
    length: &crate::css::style_compute::FfiLengthResolutionContext,
    scheme: u8,
) -> RetainedStyleValueData {
    // The host publishes what it computed for the registration itself, against the document. That
    // is what every reader of an initial value gets from the host, so it is what this answers too.
    if let Some(computed_initial) = registration.computed_initial.as_ref() {
        return computed_initial.clone();
    }
    let Some(source) = registration.initial_source.as_ref() else {
        return RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid);
    };
    let mut random_function_index = 0;
    let Some(parsed) = parse_with_syntax(
        &registry.parse_context(&mut random_function_index),
        source,
        &registration.syntax,
    ) else {
        return RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid);
    };
    absolutize_registered_custom_property_value(registry, parsed, length, None, &[], scheme).0
}

fn absolutize_registered_custom_property_value(
    registry: &CustomPropertyRegistry,
    value: StyleValueData,
    length: &crate::css::style_compute::FfiLengthResolutionContext,
    environment: Option<&crate::css::style_compute::FfiStyleComputationEnvironment>,
    random_base_values: &[crate::css::style_compute::FfiRandomBaseValue],
    scheme: u8,
) -> (RetainedStyleValueData, bool) {
    let tree_counting = environment
        .filter(|environment| environment.has_tree_counting_context)
        .map(|environment| (environment.sibling_count, environment.sibling_index));
    let context = crate::css::absolutize::AbsolutizationContext {
        length,
        scheme: Some(scheme),
        resolved_viewport_relative_length: Cell::new(false),
        tree_counting,
        random_base_values,
        document_base_url: &registry.document_base_url,
        style_sheet_resource_context: None,
    };
    let value = match crate::css::absolutize::absolutize(&value, &context) {
        Some(crate::css::absolutize::Absolutized::Changed(value)) => value,
        Some(crate::css::absolutize::Absolutized::Unchanged) | None => RetainedStyleValueData::from_owned(value),
    };
    (value, context.resolved_viewport_relative_length.get())
}

fn random_base_values_for_reparsed_value(
    reparsed: &StyleValueData,
    sources: &[&StyleValueData],
    environment: Option<&crate::css::style_compute::FfiStyleComputationEnvironment>,
) -> Vec<crate::css::style_compute::FfiRandomBaseValue> {
    let Some(environment) = environment else {
        return Vec::new();
    };
    let published = if environment.random_base_value_count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(environment.random_base_values, environment.random_base_value_count) }
    };
    let mut reparsed_sharings = Vec::new();
    crate::css::style_compute::collect_unfixed_random_sharings_in_value(reparsed, &mut reparsed_sharings);
    let source_sharings = sources
        .iter()
        .map(|source| {
            let mut sharings = Vec::new();
            crate::css::style_compute::collect_unfixed_random_sharings_in_value(source, &mut sharings);
            sharings
        })
        .collect::<Vec<_>>();
    reparsed_sharings
        .into_iter()
        .enumerate()
        .filter_map(|(index, reparsed)| {
            let StyleValueData::RandomValueSharing {
                is_auto,
                name,
                element_shared,
                ..
            } = (unsafe { &*reparsed })
            else {
                unreachable!();
            };
            let exact_base = published.iter().find(|base| {
                let StyleValueData::RandomValueSharing {
                    is_auto: published_is_auto,
                    name: published_name,
                    element_shared: published_element_shared,
                    ..
                } = (unsafe { &*base.source.cast::<StyleValueData>() })
                else {
                    return false;
                };
                *published_is_auto == *is_auto && *published_element_shared == *element_shared && published_name == name
            });
            let source_base = || {
                source_sharings.iter().find_map(|sharings| {
                    let source = *sharings.get(index)?;
                    published.iter().find(|base| base.source == source.cast())
                })
            };
            let base = exact_base.or_else(source_base);
            let base = base?;
            Some(crate::css::style_compute::FfiRandomBaseValue {
                source: reparsed.cast(),
                value: base.value,
            })
        })
        .collect()
}

/// Finalizes one substituted custom-property value against immutable registry, parent-store,
/// length, and color-scheme inputs, without consulting the DOM or any GC-managed object.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_custom_property_value(
    registry: Option<&CustomPropertyRegistry>,
    resolved_parent: Option<&CustomPropertyStore>,
    name_raw: usize,
    name: &[u16],
    value: RetainedStyleValueData,
    specified_value: Option<&StyleValueData>,
    length: Option<&crate::css::style_compute::FfiLengthResolutionContext>,
    environment: Option<&crate::css::style_compute::FfiStyleComputationEnvironment>,
    scheme: u8,
    uses_tree_counting_function: Option<&mut bool>,
) -> (RetainedStyleValueData, bool) {
    let registration = registry.and_then(|registry| registry.registrations.get(name));
    let initial = || {
        registration.map_or_else(
            || RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid),
            |registration| registered_initial_value(registry.unwrap(), registration, length.unwrap(), scheme),
        )
    };
    let inherited = || {
        resolved_parent
            .and_then(|parent| parent.get(name_raw))
            .map(|entry| entry.value.clone())
            .unwrap_or_else(initial)
    };

    let value = match value.data() {
        StyleValueData::Keyword { keyword } if name != "result".encode_utf16().collect::<Vec<_>>() => {
            if *keyword == crate::css::css_enums::keyword::INITIAL {
                initial()
            } else if *keyword == crate::css::css_enums::keyword::INHERIT {
                inherited()
            } else if *keyword == crate::css::css_enums::keyword::UNSET {
                if registration.is_some_and(|registration| !registration.inherits) {
                    initial()
                } else {
                    inherited()
                }
            } else {
                value
            }
        }
        _ => value,
    };

    let invalid_fallback = || {
        let Some(registration) = registration else {
            return RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid);
        };
        if matches!(registration.syntax, SyntaxNode::Universal) {
            return RetainedStyleValueData::from_owned(StyleValueData::GuaranteedInvalid);
        }
        if registration.inherits { inherited() } else { initial() }
    };
    if matches!(value.data(), StyleValueData::GuaranteedInvalid) {
        return (invalid_fallback(), false);
    }
    let Some(registration) = registration else {
        return (value, false);
    };
    if matches!(registration.syntax, SyntaxNode::Universal) {
        return (value, false);
    }

    let contains_attr_tainted_values = matches!(
        value.data(),
        StyleValueData::Unresolved {
            contains_attr_tainted_values: true,
            ..
        }
    );
    let Some(source) = crate::css::serialize::serialize_resolved_style_value_to_utf16(value.data()) else {
        return (invalid_fallback(), false);
    };
    let mut random_function_index = 0;
    let value_context = FfiValueParsingContext {
        kind: FfiValueParsingContextKind::Property,
        value: crate::css::property_metadata::property_id::CUSTOM,
        secondary_value: 0,
        name: Default::default(),
    };
    let mut parse_context = registry.unwrap().parse_context(&mut random_function_index);
    parse_context.value_contexts = &raw const value_context;
    parse_context.value_context_count = 1;
    let Some(parsed) = parse_with_syntax(&parse_context, &source, &registration.syntax) else {
        return (invalid_fallback(), false);
    };
    // Parsing a registered value creates fresh random-sharing nodes, while the published random
    // bases are keyed by the corresponding nodes in the substituted value. Preserve their
    // traversal correspondence across the required serialize-and-reparse step.
    let mut random_sources = vec![value.data()];
    if let Some(specified_value) = specified_value {
        random_sources.push(specified_value);
    }
    let random_base_values = random_base_values_for_reparsed_value(&parsed, &random_sources, environment);
    if let Some(uses_tree_counting_function) = uses_tree_counting_function {
        *uses_tree_counting_function =
            crate::css::style_compute::collect_external_value_dependencies(&parsed).uses_tree_counting_function;
    }
    let (computed, depends_on_viewport_metrics) = absolutize_registered_custom_property_value(
        registry.unwrap(),
        parsed,
        length.unwrap(),
        environment,
        &random_base_values,
        scheme,
    );
    if !contains_attr_tainted_values {
        return (computed, depends_on_viewport_metrics);
    }

    let source = crate::css::serialize::serialize_resolved_style_value_to_utf16(computed.data()).unwrap_or_default();
    let mut wrapped = crate::css::parser::value_parser::unresolved_value(
        &source,
        &[],
        crate::css::parser::arbitrary_substitution::SubstitutionFunctionsPresence::default(),
    );
    let StyleValueData::Unresolved {
        contains_attr_tainted_values,
        parsed_value,
        ..
    } = &mut wrapped
    else {
        unreachable!();
    };
    *contains_attr_tainted_values = true;
    *parsed_value = computed;
    (RetainedStyleValueData::from_owned(wrapped), depends_on_viewport_metrics)
}

impl CustomPropertyStore {
    /// Filter one store layer over an already filtered parent. The returned pointer owns
    /// one reference, including when the result is the source or the parent itself.
    unsafe fn inheritable_layer(source: *const Self, parent: *const c_void, excluded: &[usize]) -> *const c_void {
        let store = unsafe { &*source };
        if store.own_values.is_empty() {
            return unsafe { Self::retained_parent(parent) }.map_or(std::ptr::null(), |p| Arc::into_raw(p).cast());
        }
        if excluded.is_empty()
            && parent
                == store
                    .parent
                    .as_ref()
                    .map_or(std::ptr::null(), |p| Arc::as_ptr(p).cast())
        {
            unsafe { Arc::increment_strong_count(source) };
            return source.cast();
        }
        let mut names = store.declared_names.clone();
        let mut absorbed: Vec<_> = store
            .own_values
            .keys()
            .filter(|name| !names.contains(name))
            .copied()
            .collect();
        absorbed.sort_unstable();
        names.extend(absorbed);
        let entries: Vec<_> = names
            .into_iter()
            .filter(|name| !excluded.contains(name))
            .filter(|name| {
                // A child store may absorb entries from its parent for lookup speed. Filtering
                // the child's own non-inheriting declarations must not redeclare those entries
                // in a new layer: they already belong to the filtered parent.
                store.declared_names.contains(name)
                    || unsafe { parent.cast::<Self>().as_ref() }.is_none_or(|parent| {
                        parent.get(*name).is_none_or(|inherited| {
                            let entry = &store.own_values[name];
                            inherited.value.pointer() != entry.value.pointer() || inherited.important != entry.important
                        })
                    })
            })
            .map(|name| (name, store.own_values[&name].clone()))
            .collect();
        if entries.is_empty() {
            return unsafe { Self::retained_parent(parent) }.map_or(std::ptr::null(), |p| Arc::into_raw(p).cast());
        }
        Self::child(unsafe { Self::retained_parent(parent) }, entries)
    }

    /// The environment inherited by a child, with non-inheriting registrations removed.
    /// Registration generations are part of the caller's memo key.
    pub(crate) unsafe fn inheritable(source: *const Self, registry: &CustomPropertyRegistry) -> *const c_void {
        let store = unsafe { &*source };
        let parent = store.parent.as_ref().map_or(std::ptr::null(), |parent| unsafe {
            Self::inheritable(Arc::as_ptr(parent), registry)
        });
        let excluded: Vec<_> = store
            .own_values
            .iter()
            .filter_map(|(&name, entry)| {
                registry
                    .registrations
                    .get(entry.name.as_ref())
                    .is_some_and(|registration| !registration.inherits)
                    .then_some(name)
            })
            .collect();
        let result = unsafe { Self::inheritable_layer(source, parent, &excluded) };
        if !parent.is_null() {
            unsafe { Arc::decrement_strong_count(parent.cast::<Self>()) };
        }
        result
    }

    /// Whether this store resolves every custom property to the same value as `other`, such as a
    /// copy the host made of an environment the engine resolved.
    pub(crate) fn resolves_like(&self, other: &Self) -> bool {
        let mut names = std::collections::HashSet::new();
        for mut store in [self, other] {
            loop {
                names.extend(store.own_values.keys().copied());
                let Some(parent) = store.parent.as_deref() else {
                    break;
                };
                store = parent;
            }
        }
        names.into_iter().all(|name| match (self.get(name), other.get(name)) {
            (Some(ours), Some(theirs)) => ours.value == theirs.value && ours.important == theirs.important,
            (None, None) => true,
            _ => false,
        })
    }

    pub(crate) fn get(&self, name_raw: usize) -> Option<&CustomPropertyEntry> {
        self.own_values
            .get(&name_raw)
            .or_else(|| self.parent.as_ref()?.get(name_raw))
    }

    pub(crate) fn get_named(&self, name: &[u16]) -> Option<(usize, &CustomPropertyEntry)> {
        if let Some(&raw) = self.own_names.get(name) {
            return self.own_values.get(&raw).map(|entry| (raw, entry));
        }
        self.parent.as_ref()?.get_named(name)
    }

    fn get_by_name_with_owner(&self, name: &[u16]) -> Option<(&CustomPropertyEntry, &CustomPropertyStore)> {
        self.own_names
            .get(name)
            .and_then(|name_raw| self.own_values.get(name_raw))
            .map(|entry| (entry, self))
            .or_else(|| self.parent.as_ref()?.get_by_name_with_owner(name))
    }

    fn get_own_by_name(&self, name: &[u16]) -> Option<&CustomPropertyEntry> {
        self.own_names
            .get(name)
            .and_then(|name_raw| self.own_values.get(name_raw))
    }

    /// The value this store answers for a name, retained for a caller that outlives the borrow.
    pub(crate) fn retained_value(&self, name_raw: usize) -> Option<RetainedStyleValueData> {
        self.get(name_raw).map(|entry| entry.value.clone_retained())
    }

    /// Whether the element this store belongs to declares `name_raw` itself, with `!important`.
    /// Only the declared prefix counts: the rest of `own_values` is what structural sharing
    /// absorbed from ancestors, which the element did not declare.
    pub(crate) fn declares_important(&self, name_raw: usize) -> bool {
        self.declared_names.contains(&name_raw) && self.own_values.get(&name_raw).is_some_and(|entry| entry.important)
    }

    pub(crate) fn value_matches(&self, name_raw: usize, value: &StyleValueData) -> bool {
        self.get(name_raw).is_some_and(|entry| entry.value.data() == value)
    }

    pub(crate) fn value_is_identical(&self, name_raw: usize, value: *const c_void) -> bool {
        self.get(name_raw)
            .is_some_and(|entry| entry.value.pointer().cast() == value)
    }

    unsafe fn retained_parent(parent: *const c_void) -> Option<Arc<CustomPropertyStore>> {
        if parent.is_null() {
            return None;
        }
        let parent = parent.cast::<CustomPropertyStore>();
        unsafe { Arc::increment_strong_count(parent) };
        Some(unsafe { Arc::from_raw(parent) })
    }

    fn child(
        mut parent: Option<Arc<CustomPropertyStore>>,
        entries: Vec<(usize, CustomPropertyEntry)>,
    ) -> *const c_void {
        let mut own_names = HashMap::with_capacity(entries.len());
        let mut own_values = HashMap::with_capacity(entries.len());
        let mut declared_names = Vec::with_capacity(entries.len());
        for (name_raw, entry) in entries {
            declared_names.push(name_raw);
            own_names.insert(entry.name.clone(), name_raw);
            own_values.insert(name_raw, entry);
        }

        let inheritance_parent = parent.clone();
        let ancestor_count = if let Some(current_parent) = parent.clone() {
            if current_parent.ancestor_count >= MAX_ANCESTOR_COUNT - 1 {
                let mut ancestor = Some(current_parent.as_ref());
                while let Some(current) = ancestor {
                    for (&name_raw, entry) in &current.own_values {
                        if let std::collections::hash_map::Entry::Vacant(slot) = own_values.entry(name_raw) {
                            own_names.insert(entry.name.clone(), name_raw);
                            slot.insert(entry.clone());
                        }
                    }
                    ancestor = current.parent.as_deref();
                }
                parent = None;
                0
            } else if current_parent.own_values.len() <= ABSORB_THRESHOLD {
                for (&name_raw, entry) in &current_parent.own_values {
                    if let std::collections::hash_map::Entry::Vacant(slot) = own_values.entry(name_raw) {
                        own_names.insert(entry.name.clone(), name_raw);
                        slot.insert(entry.clone());
                    }
                }
                parent = current_parent.parent.clone();
                parent.as_ref().map_or(0, |parent| parent.ancestor_count + 1)
            } else {
                current_parent.ancestor_count + 1
            }
        } else {
            0
        };

        Arc::into_raw(Arc::new(CustomPropertyStore {
            own_values,
            declared_names,
            own_names,
            inheritance_parent,
            parent,
            ancestor_count,
        }))
        .cast()
    }

    /// # Safety
    /// `parent` must be null or a live raw `Arc` pointer to a `CustomPropertyStore`.
    pub(crate) unsafe fn resolved_child(
        &self,
        parent: *const c_void,
        values: Vec<(usize, RetainedStyleValueData)>,
    ) -> *const c_void {
        let entries = values
            .into_iter()
            .map(|(name_raw, value)| {
                let source = self
                    .own_values
                    .get(&name_raw)
                    .expect("resolved custom property must be an own value");
                (
                    name_raw,
                    CustomPropertyEntry {
                        _name: source._name.clone(),
                        name: source.name.clone(),
                        value,
                        important: source.important,
                    },
                )
            })
            .collect();
        Self::child(unsafe { Self::retained_parent(parent) }, entries)
    }

    /// # Safety
    /// `parent` must be null or a live raw `Arc` pointer, and every value pointer must remain live
    /// for this call.
    pub(crate) unsafe fn cascaded_child(
        parent: *const c_void,
        values: Vec<(usize, Arc<[u16]>, bool, *const c_void)>,
    ) -> *const c_void {
        let entries = values
            .into_iter()
            .map(|(name_raw, name, important, value)| {
                (
                    name_raw,
                    CustomPropertyEntry {
                        _name: unsafe { RetainedUtf16FlyString::from_borrowed_raw(name_raw) },
                        name,
                        value: unsafe {
                            RetainedStyleValueData::from_retained_pointer(
                                crate::css::style_value::retain_style_value(value.cast()).cast(),
                            )
                        },
                        important,
                    },
                )
            })
            .collect();
        Self::child(unsafe { Self::retained_parent(parent) }, entries)
    }

    /// Whether `store` holds values of its own over exactly `parent`, a null `parent` being none.
    ///
    /// # Safety
    /// `store` must be a live store.
    pub(crate) unsafe fn is_composed_over(store: *const c_void, parent: *const c_void) -> bool {
        let store = unsafe { &*store.cast::<CustomPropertyStore>() };
        store
            .parent
            .as_ref()
            .map_or(std::ptr::null(), |store_parent| Arc::as_ptr(store_parent).cast())
            == parent
    }
}

const MAX_SUBSTITUTED_TOKEN_COUNT: usize = 16384;
const MAX_SUBSTITUTION_RECURSION_DEPTH: u32 = 64;

pub(crate) enum NativeVarResolution {
    Resolved {
        source: Vec<u16>,
        contains_attr_tainted_values: bool,
    },
    Invalid,
    NotHandled,
}

enum TokenResolution {
    Resolved(Vec<OwnedToken>),
    Invalid,
    Cyclic,
    NotHandled,
}

// https://drafts.csswg.org/css-values-5/#substitution-context
#[derive(PartialEq, Eq)]
enum SubstitutionContextDependency {
    Property(Vec<u16>, Option<CustomFunctionIdentity>),
    // FIXME: Attribute substitution context equality should compare names ASCII-case-insensitively, as attribute lookup
    // does.
    Attribute(Vec<u16>),
    Function(CustomFunctionIdentity),
}

impl SubstitutionContextDependency {
    fn is_property(&self, name: &[u16], custom_function: Option<CustomFunctionIdentity>) -> bool {
        matches!(self, Self::Property(property_name, property_custom_function) if property_name == name && *property_custom_function == custom_function)
    }
}

struct SubstitutionContext {
    dependency: SubstitutionContextDependency,
    is_cyclic: Cell<bool>,
}

impl SubstitutionContext {
    fn new(dependency: SubstitutionContextDependency) -> Rc<Self> {
        Rc::new(Self {
            dependency,
            is_cyclic: Cell::new(false),
        })
    }
}

#[derive(Default)]
struct GuardedSubstitutionContexts {
    contexts: Rc<RefCell<Vec<Rc<SubstitutionContext>>>>,
}

impl GuardedSubstitutionContexts {
    // https://drafts.csswg.org/css-values-5/#guarded
    fn guard(&self, context: &Rc<SubstitutionContext>) -> Option<ScopeGuard<impl FnMut() + use<>>> {
        if self.mark_cycle_if_guarded(&context.dependency) {
            context.is_cyclic.set(true);
            return None;
        }

        self.contexts.borrow_mut().push(Rc::clone(context));

        let contexts = Rc::clone(&self.contexts);
        let context = Rc::clone(context);
        Some(ScopeGuard::new(move || {
            let popped = contexts.borrow_mut().pop().expect("guarded substitution context");

            assert!(Rc::ptr_eq(&popped, &context));
        }))
    }

    fn mark_cycle_if_guarded(&self, dependency: &SubstitutionContextDependency) -> bool {
        let contexts = self.contexts.borrow();
        let Some(cycle_start) = contexts.iter().position(|context| &context.dependency == dependency) else {
            return false;
        };
        for context in &contexts[cycle_start..] {
            context.is_cyclic.set(true);
        }
        true
    }

    fn innermost_function(&self) -> Option<CustomFunctionIdentity> {
        self.contexts
            .borrow()
            .iter()
            .rev()
            .find_map(|context| match &context.dependency {
                SubstitutionContextDependency::Function(identity) => Some(*identity),
                _ => None,
            })
    }
}

#[derive(Default)]
struct ASFResolutionContext<'a> {
    guarded_contexts: GuardedSubstitutionContexts,
    attributes: Option<&'a HashMap<Vec<u16>, Vec<u16>>>,
    inheritance_store: Option<&'a CustomPropertyStore>,
    inheritance_store_overrides: Vec<Option<Arc<CustomPropertyStore>>>,
    attribute_names_are_ascii_case_insensitive: bool,
    contains_attr_tainted_values: bool,
    custom_functions: Option<&'a CustomFunctionRegistry>,
    parse_context: Option<&'a ParseContext>,
    media_environment: Option<&'a FfiMediaEnvironment>,
    style_query_length_resolution_context: Option<&'a crate::css::style_compute::FfiLengthResolutionContext>,
    style_query_color_resolution_input: Option<crate::css::color_resolution::ColorResolutionInput<'a>>,
    style_query_tree_counting: Option<(u64, u64)>,
    style_query_dependencies: Option<&'a mut crate::css::cascaded_properties::StyleQueryDependencies>,
    final_custom_properties: Option<&'a HashMap<Vec<u16>, *const c_void>>,
    function_local_scopes: Vec<FunctionLocalScope>,
    token_cache: Option<&'a mut CustomPropertyTokenCache>,
    resolution_stats: Option<&'a VarResolutionStats>,
}

impl ASFResolutionContext<'_> {
    fn media_environment(&mut self) -> Option<&FfiMediaEnvironment> {
        self.media_environment
    }
}

type CustomPropertyTokenCache = HashMap<*const StyleValueData, (Arc<[OwnedToken]>, bool, bool)>;

pub(crate) struct VarResolutionEnvironment {
    attributes: HashMap<Vec<u16>, Vec<u16>>,
    custom_functions: CustomFunctionRegistry,
    token_cache: CustomPropertyTokenCache,
    resolution_stats: VarResolutionStats,
}

#[derive(Default)]
struct VarResolutionStats {
    final_value_hits: std::cell::Cell<u64>,
    final_value_misses: std::cell::Cell<u64>,
}

impl VarResolutionEnvironment {
    pub(crate) fn final_value_hits(&self) -> u64 {
        self.resolution_stats.final_value_hits.get()
    }

    pub(crate) fn final_value_misses(&self) -> u64 {
        self.resolution_stats.final_value_misses.get()
    }
}

fn matching_close(kind: &OwnedTokenKind) -> Option<OwnedTokenKind> {
    match kind {
        OwnedTokenKind::Function(_) | OwnedTokenKind::OpenParen => Some(OwnedTokenKind::CloseParen),
        OwnedTokenKind::OpenSquare => Some(OwnedTokenKind::CloseSquare),
        OwnedTokenKind::OpenCurly => Some(OwnedTokenKind::CloseCurly),
        _ => None,
    }
}

fn find_matching_close(tokens: &[OwnedToken], open_index: usize) -> Option<usize> {
    let mut expected_closes = vec![matching_close(&tokens[open_index].kind)?];
    for (index, token) in tokens.iter().enumerate().skip(open_index + 1) {
        if let Some(close) = matching_close(&token.kind) {
            expected_closes.push(close);
            continue;
        }
        if expected_closes.last() == Some(&token.kind) {
            expected_closes.pop();
            if expected_closes.is_empty() {
                return Some(index);
            }
        }
    }
    None
}

fn find_top_level_comma(tokens: &[OwnedToken]) -> Option<usize> {
    let mut index = 0;
    while index < tokens.len() {
        if matches!(tokens[index].kind, OwnedTokenKind::Comma) {
            return Some(index);
        }
        if matching_close(&tokens[index].kind).is_some() {
            index = find_matching_close(tokens, index)? + 1;
        } else {
            index += 1;
        }
    }
    None
}

fn find_top_level_source(tokens: &[OwnedToken], source: &[u8]) -> Option<usize> {
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].source.equals_ascii(source) {
            return Some(index);
        }
        if matching_close(&tokens[index].kind).is_some() {
            index = find_matching_close(tokens, index)? + 1;
        } else {
            index += 1;
        }
    }
    None
}

fn trim_whitespace(mut tokens: &[OwnedToken]) -> &[OwnedToken] {
    while matches!(
        tokens.first().map(|token| &token.kind),
        Some(OwnedTokenKind::Whitespace)
    ) {
        tokens = &tokens[1..];
    }
    while matches!(tokens.last().map(|token| &token.kind), Some(OwnedTokenKind::Whitespace)) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

fn source_ends_with_comment(source: crate::css::css_tokenizer::TokenizerInput<'_>) -> bool {
    match source {
        crate::css::css_tokenizer::TokenizerInput::Ascii(source) => source
            .iter()
            .rposition(|unit| !unit.is_ascii_whitespace())
            .is_some_and(|end| end > 0 && source[end - 1..=end] == *b"*/"),
        crate::css::css_tokenizer::TokenizerInput::Utf16(source) => source
            .iter()
            .rposition(|unit| !matches!(*unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
            .is_some_and(|end| end > 0 && source[end - 1..=end] == [u16::from(b'*'), u16::from(b'/')]),
    }
}

fn is_single_css_wide_keyword(tokens: &[OwnedToken]) -> bool {
    let [
        OwnedToken {
            kind: OwnedTokenKind::Ident(keyword),
            ..
        },
    ] = trim_whitespace(tokens)
    else {
        return false;
    };
    keyword.eq_ignore_ascii_case("inherit")
        || keyword.eq_ignore_ascii_case("initial")
        || keyword.eq_ignore_ascii_case("unset")
        || keyword.eq_ignore_ascii_case("revert")
        || keyword.eq_ignore_ascii_case("revert-layer")
}

fn single_css_wide_keyword(tokens: &[OwnedToken]) -> Option<&[u16]> {
    let [
        OwnedToken {
            kind: OwnedTokenKind::Ident(keyword),
            ..
        },
    ] = trim_whitespace(tokens)
    else {
        return None;
    };
    is_single_css_wide_keyword(tokens).then_some(keyword.as_slice())
}

fn tokens_for_custom_property_value(data: &StyleValueData) -> Option<(Vec<OwnedToken>, bool, bool)> {
    if let StyleValueData::Unresolved {
        presence_attr,
        presence_dashed_function,
        presence_env,
        presence_if,
        presence_inherit,
        presence_var,
        contains_attr_tainted_values,
        ..
    } = data
    {
        let mut tokens = tokenize_owned(data.unresolved_token_source().unwrap_or_default());
        let authored_source = data.unresolved_authored_source().unwrap_or_default();
        if source_ends_with_comment(authored_source) {
            while matches!(tokens.last().map(|token| &token.kind), Some(OwnedTokenKind::Whitespace)) {
                tokens.pop();
            }
        }
        return Some((
            tokens,
            *presence_var
                || *presence_attr
                || *presence_dashed_function
                || *presence_env
                || *presence_if
                || *presence_inherit,
            *contains_attr_tainted_values,
        ));
    }
    let source = crate::css::serialize::serialize_style_value_to_utf16(data)?;
    Some((tokenize_owned(&source), false, false))
}

fn cached_tokens_for_custom_property_value(
    data: &StyleValueData,
    context: &mut ASFResolutionContext<'_>,
) -> Option<(Arc<[OwnedToken]>, bool, bool)> {
    let key = std::ptr::from_ref(data);
    if let Some(cached) = context.token_cache.as_deref().and_then(|cache| cache.get(&key)) {
        return Some(cached.clone());
    }
    let (tokens, includes_substitution, contains_attr_tainted_values) = tokens_for_custom_property_value(data)?;
    let result = (Arc::from(tokens), includes_substitution, contains_attr_tainted_values);
    if let Some(cache) = context.token_cache.as_deref_mut() {
        cache.insert(key, result.clone());
    }
    Some(result)
}

fn tokens_for_function_value(data: &StyleValueData) -> Option<(Vec<OwnedToken>, bool)> {
    if let StyleValueData::Unresolved {
        presence_attr,
        presence_dashed_function,
        presence_env,
        presence_if,
        presence_inherit,
        presence_var,
        ..
    } = data
    {
        return Some((
            tokenize_owned(data.unresolved_token_source().unwrap_or_default()),
            *presence_var
                || *presence_attr
                || *presence_dashed_function
                || *presence_env
                || *presence_if
                || *presence_inherit,
        ));
    }
    let source = crate::css::serialize::serialize_style_value_to_utf16(data)?;
    Some((tokenize_owned(&source), false))
}

unsafe fn custom_function_registry_from_ffi(
    definitions: *const FfiSubstitutionFunctionDefinition,
    definition_count: usize,
    caller_scope_identity: usize,
    visibilities: *const FfiSubstitutionFunctionVisibility,
    visibility_count: usize,
) -> Option<CustomFunctionRegistry> {
    let definitions = if definition_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(definitions, definition_count) }
    };
    let mut parsed_definitions = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let signature = definition.signature.cast::<FunctionSignature>();
        if signature.is_null() {
            return None;
        }
        let signature = unsafe {
            Arc::increment_strong_count(signature);
            Arc::from_raw(signature)
        };
        let mut parameter_defaults = Vec::with_capacity(signature.parameters.len());
        for parameter in &signature.parameters {
            parameter_defaults.push(match &parameter.default_value {
                Some(data) => Some(tokens_for_function_value(data)?.0),
                None => None,
            });
        }
        let declarations = if definition.declaration_count == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(definition.declarations, definition.declaration_count) }
        };
        let mut parsed_declarations = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let name = unsafe { declaration.name.to_utf16() }?;
            let data = unsafe { &*declaration.data.cast::<StyleValueData>() };
            let (tokens, includes_substitution) = tokens_for_function_value(data)?;
            parsed_declarations.push((name, tokens, includes_substitution));
        }
        parsed_definitions.push(CustomFunctionDefinition {
            identity: definition.identity,
            scope_identity: definition.scope_identity,
            signature,
            parameter_defaults,
            declarations: parsed_declarations,
        });
    }
    let visibilities = if visibility_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(visibilities, visibility_count) }
    };
    let mut visible_definitions = HashMap::with_capacity(visibilities.len());
    for visibility in visibilities {
        let definition = parsed_definitions
            .iter()
            .find(|definition| definition.identity == visibility.function_identity)?;
        visible_definitions.insert(
            (
                visibility.caller_scope_identity,
                definition.signature.name.units().to_vec(),
            ),
            definition.identity,
        );
    }
    Some(CustomFunctionRegistry {
        caller_scope_identity,
        definitions: parsed_definitions,
        visible_definitions,
    })
}

/// Builds the element-wide immutable substitution inputs once for all declarations in a cascade.
///
/// # Safety
/// Every FFI pointer must remain readable for this call.
pub(crate) unsafe fn prepare_var_resolution_environment(
    attributes: *const FfiSubstitutionAttribute,
    attribute_count: usize,
    custom_functions: *const FfiSubstitutionFunctionDefinition,
    custom_function_count: usize,
    custom_function_scope_identity: usize,
    custom_function_visibilities: *const FfiSubstitutionFunctionVisibility,
    custom_function_visibility_count: usize,
) -> Option<VarResolutionEnvironment> {
    let attributes = if attribute_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(attributes, attribute_count) }
    };
    let attributes = attributes
        .iter()
        .filter_map(|attribute| {
            Some((unsafe { attribute.name.to_utf16() }?, unsafe {
                attribute.value.to_utf16()
            }?))
        })
        .collect();
    let custom_functions = unsafe {
        custom_function_registry_from_ffi(
            custom_functions,
            custom_function_count,
            custom_function_scope_identity,
            custom_function_visibilities,
            custom_function_visibility_count,
        )
    }?;
    Some(VarResolutionEnvironment {
        attributes,
        custom_functions,
        token_cache: HashMap::new(),
        resolution_stats: VarResolutionStats::default(),
    })
}

fn registration_accepts_tokens(
    registry: &CustomPropertyRegistry,
    registration: &RegisteredCustomProperty,
    tokens: &[OwnedToken],
) -> bool {
    if matches!(registration.syntax, SyntaxNode::Universal) {
        return true;
    }
    let source = serialize_tokens(tokens);
    let mut random_function_index = 0;
    let value_context = FfiValueParsingContext {
        kind: FfiValueParsingContextKind::Property,
        value: crate::css::property_metadata::property_id::CUSTOM,
        secondary_value: 0,
        name: Default::default(),
    };
    let mut context = registry.parse_context(&mut random_function_index);
    context.value_contexts = &raw const value_context;
    context.value_context_count = 1;
    parse_with_syntax(&context, &source, &registration.syntax).is_some()
}

fn registered_property_fallback(
    inheritance_store: Option<&CustomPropertyStore>,
    registry: &CustomPropertyRegistry,
    registration: &RegisteredCustomProperty,
    name: &[u16],
    recursion_depth: u32,
) -> TokenResolution {
    if registration.inherits
        && let Some(parent) = inheritance_store
    {
        return resolve_custom_property_with_lookup(
            Some(parent),
            Some(registry),
            name,
            &mut ASFResolutionContext::default(),
            recursion_depth + 1,
            CustomPropertyLookup::ExplicitInheritance,
        );
    }
    registration
        .initial_source
        .as_ref()
        .map_or(TokenResolution::Invalid, |source| {
            TokenResolution::Resolved(tokenize_owned(source))
        })
}

fn resolve_css_wide_keyword(
    owner: &CustomPropertyStore,
    registry: Option<&CustomPropertyRegistry>,
    name: &[u16],
    keyword: &[u16],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
    lookup: CustomPropertyLookup,
) -> TokenResolution {
    let registration = registry.and_then(|registry| registry.registrations.get(name));
    // Mirror StyleComputer::resolve_css_wide_keyword_for_custom_property(). Revert keywords
    // remain unresolved there pending custom-property revert support; typed `revert` then reaches
    // the invalid-value fallback below.
    if keyword.eq_ignore_ascii_case("initial") {
        return registration
            .and_then(|registration| registration.initial_source.as_ref())
            .map_or(TokenResolution::Invalid, |source| {
                TokenResolution::Resolved(tokenize_owned(source))
            });
    }
    if keyword.eq_ignore_ascii_case("inherit")
        || keyword.eq_ignore_ascii_case("unset") && registration.is_none_or(|registration| registration.inherits)
    {
        let inheritance_store = match lookup {
            CustomPropertyLookup::Normal => context.inheritance_store,
            CustomPropertyLookup::ExplicitInheritance => owner.inheritance_parent.as_deref(),
        };
        return resolve_custom_property_with_lookup(
            inheritance_store,
            registry,
            name,
            context,
            recursion_depth + 1,
            CustomPropertyLookup::ExplicitInheritance,
        );
    }
    if keyword.eq_ignore_ascii_case("unset") {
        return registration
            .and_then(|registration| registration.initial_source.as_ref())
            .map_or(TokenResolution::Invalid, |source| {
                TokenResolution::Resolved(tokenize_owned(source))
            });
    }
    // NB: Typed registered `revert` uses the invalid-value fallback, while `revert-layer` remains
    //     unresolved pending custom-property revert support.
    if keyword.eq_ignore_ascii_case("revert")
        && let (Some(registry), Some(registration)) = (registry, registration)
        && !matches!(registration.syntax, SyntaxNode::Universal)
    {
        let inheritance_store = match lookup {
            CustomPropertyLookup::Normal => context.inheritance_store,
            CustomPropertyLookup::ExplicitInheritance => owner.inheritance_parent.as_deref(),
        };
        return registered_property_fallback(inheritance_store, registry, registration, name, recursion_depth);
    }
    TokenResolution::Resolved(tokenize_owned(crate::css::css_tokenizer::TokenizerInput::Utf16(
        keyword,
    )))
}

fn normalize_function_tokens(
    registry: Option<&CustomPropertyRegistry>,
    syntax: &SyntaxNode,
    tokens: &[OwnedToken],
) -> TokenResolution {
    if matches!(syntax, SyntaxNode::Universal) {
        return TokenResolution::Resolved(tokens.to_vec());
    }
    let Some(registry) = registry else {
        return TokenResolution::NotHandled;
    };
    let source = serialize_tokens(tokens);
    let mut random_function_index = 0;
    let context = registry.parse_context(&mut random_function_index);
    let Some(parsed) = parse_with_syntax(&context, &source, syntax) else {
        return TokenResolution::Invalid;
    };
    let computed = if matches!(parsed, StyleValueData::Calculated { .. }) {
        crate::css::calc::collapse_calculated_without_context(&parsed)
    } else {
        None
    };
    let Some(source) =
        crate::css::serialize::serialize_resolved_style_value_to_utf16(computed.as_ref().unwrap_or(&parsed))
    else {
        return TokenResolution::Invalid;
    };
    TokenResolution::Resolved(tokenize_owned(&source))
}

#[allow(clippy::too_many_arguments)]
fn resolve_function_local_property(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    name: &[u16],
    function_identity: CustomFunctionIdentity,
    value: Option<FunctionLocalValue>,
    registration: FunctionLocalRegistration,
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    let Some(value) = value else {
        return registration
            .initial_tokens
            .map_or(TokenResolution::Invalid, TokenResolution::Resolved);
    };
    let mut result = if value.includes_substitution {
        substitute_arbitrary_substitution_functions(
            store,
            registry,
            &value.tokens,
            context,
            recursion_depth + 1,
            Some(SubstitutionContextDependency::Property(
                name.to_owned(),
                Some(function_identity),
            )),
        )
    } else {
        TokenResolution::Resolved(value.tokens)
    };
    // https://drafts.csswg.org/css-mixins/#resolve-function-styles
    // On result, all CSS-wide keywords are left unresolved.
    if !registration.is_result
        && let TokenResolution::Resolved(tokens) = &result
        && let Some(keyword) = single_css_wide_keyword(tokens)
    {
        if keyword.eq_ignore_ascii_case("initial") {
            return registration
                .initial_tokens
                .map_or(TokenResolution::Invalid, TokenResolution::Resolved);
        }
        if keyword.eq_ignore_ascii_case("inherit") {
            let local_scope = context
                .function_local_scopes
                .pop()
                .expect("function-local property scope");
            result = resolve_custom_property(store, registry, name, context, recursion_depth + 1);
            context.function_local_scopes.push(local_scope);
        } else {
            return TokenResolution::Invalid;
        }
    }
    match result {
        TokenResolution::Resolved(tokens) => normalize_function_tokens(registry, &registration.syntax, &tokens),
        other => other,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CustomPropertyLookup {
    Normal,
    ExplicitInheritance,
}

fn resolve_custom_property(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    name: &[u16],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    resolve_custom_property_with_lookup(
        store,
        registry,
        name,
        context,
        recursion_depth,
        CustomPropertyLookup::Normal,
    )
}

fn resolve_custom_property_with_lookup(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    name: &[u16],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
    lookup: CustomPropertyLookup,
) -> TokenResolution {
    let local_scope_index = context
        .function_local_scopes
        .iter()
        .rposition(|local_scope| local_scope.values.contains_key(name) || local_scope.registrations.contains_key(name));
    if let Some(local_scope_index) = local_scope_index {
        let function_scope = &context.function_local_scopes[local_scope_index];

        if let Some(cached_value) = function_scope.resolved_value_cache.borrow().get(name) {
            return TokenResolution::Resolved(cached_value.clone());
        }

        let function_identity = function_scope.function_identity;
        let value = function_scope.values.get(name).cloned();
        let registration = function_scope.registrations.get(name).cloned();
        let child_scopes = context.function_local_scopes.split_off(local_scope_index + 1);
        let result = resolve_function_local_property(
            store,
            registry,
            name,
            function_identity,
            value,
            registration.unwrap_or(FunctionLocalRegistration {
                syntax: SyntaxNode::Universal,
                initial_tokens: None,
                is_result: false,
            }),
            context,
            recursion_depth,
        );
        context.function_local_scopes.extend(child_scopes);

        if let TokenResolution::Resolved(tokens) = &result {
            context.function_local_scopes[local_scope_index]
                .resolved_value_cache
                .borrow_mut()
                .insert(name.to_owned(), tokens.clone());
        }

        return result;
    }
    let substitution_context = SubstitutionContextDependency::Property(name.to_owned(), None);
    // AD-HOC: The root custom property's unresolved value is passed directly to `resolve_vars()` rather than read
    //         through this lookup. If it contains a `var()` that refers back to itself, this function is entered while
    //         the caller's root context is already guarded, so mark that context cyclic before consulting stored or
    //         cached values.
    if lookup == CustomPropertyLookup::Normal && context.guarded_contexts.mark_cycle_if_guarded(&substitution_context) {
        return TokenResolution::Cyclic;
    }
    if lookup == CustomPropertyLookup::Normal
        && let Some(final_values) = context.final_custom_properties
    {
        if let Some(data) = final_values
            .get(name)
            .and_then(|data| unsafe { data.cast::<StyleValueData>().as_ref() })
        {
            if let Some(stats) = context.resolution_stats {
                stats.final_value_hits.set(stats.final_value_hits.get() + 1);
            }
            if matches!(data, StyleValueData::GuaranteedInvalid) {
                return TokenResolution::Invalid;
            }
            let Some((tokens, includes_substitution, contains_attr_tainted_values)) =
                cached_tokens_for_custom_property_value(data, context)
            else {
                return TokenResolution::NotHandled;
            };
            context.contains_attr_tainted_values |= contains_attr_tainted_values;
            if !includes_substitution {
                return TokenResolution::Resolved(tokens.to_vec());
            }
        } else if let Some(stats) = context.resolution_stats {
            stats.final_value_misses.set(stats.final_value_misses.get() + 1);
        }
    }
    let registration = registry.and_then(|registry| registry.registrations.get(name));
    let entry_and_owner = store.and_then(|store| {
        if lookup == CustomPropertyLookup::ExplicitInheritance {
            return store.get_by_name_with_owner(name);
        }
        match registration {
            Some(registration) if !registration.inherits => store.get_own_by_name(name).map(|entry| (entry, store)),
            _ => store.get_by_name_with_owner(name),
        }
    });
    let Some((entry, owner)) = entry_and_owner else {
        return registration
            .and_then(|registration| registration.initial_source.as_ref())
            .map_or(TokenResolution::Invalid, |source| {
                TokenResolution::Resolved(tokenize_owned(source))
            });
    };
    let data = entry.value.data();
    if matches!(data, StyleValueData::GuaranteedInvalid) {
        return TokenResolution::Invalid;
    }
    let Some((source, includes_var, contains_attr_tainted_values)) =
        cached_tokens_for_custom_property_value(data, context)
    else {
        return TokenResolution::NotHandled;
    };
    context.contains_attr_tainted_values |= contains_attr_tainted_values;
    if lookup == CustomPropertyLookup::ExplicitInheritance {
        context
            .inheritance_store_overrides
            .push(owner.inheritance_parent.clone());
    }
    let result = if includes_var {
        substitute_arbitrary_substitution_functions(
            store,
            registry,
            &source,
            context,
            recursion_depth + 1,
            Some(substitution_context),
        )
    } else {
        TokenResolution::Resolved(source.to_vec())
    };
    if lookup == CustomPropertyLookup::ExplicitInheritance {
        context.inheritance_store_overrides.pop();
    }
    if let TokenResolution::Resolved(tokens) = &result
        && let Some(keyword) = single_css_wide_keyword(tokens)
    {
        return resolve_css_wide_keyword(owner, registry, name, keyword, context, recursion_depth, lookup);
    }
    if let (Some(registry), Some(registration)) = (registry, registration) {
        let inheritance_store = match lookup {
            CustomPropertyLookup::Normal => context.inheritance_store,
            CustomPropertyLookup::ExplicitInheritance => owner.inheritance_parent.as_deref(),
        };
        return match result {
            TokenResolution::Resolved(tokens) if registration_accepts_tokens(registry, registration, &tokens) => {
                TokenResolution::Resolved(tokens)
            }
            TokenResolution::Resolved(_) | TokenResolution::Invalid | TokenResolution::Cyclic => {
                registered_property_fallback(inheritance_store, registry, registration, name, recursion_depth)
            }
            TokenResolution::NotHandled => TokenResolution::NotHandled,
        };
    }
    result
}

fn replace_var_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // https://drafts.csswg.org/css-variables-1/#replace-a-var-function
    // 1. Let el be the element that the style containing the var() function is being applied to.
    //    Let first arg be the first <declaration-value> in arguments.
    //    Let second arg be the <declaration-value>? passed after the comma, or null if there was no comma.
    let comma = find_top_level_comma(arguments);
    let first_argument = &arguments[..comma.unwrap_or(arguments.len())];

    // 2. Substitute arbitrary substitution functions in first arg, then parse it as a <custom-property-name>.
    //    If parsing returned a <custom-property-name>, let result be the computed value of the corresponding custom
    //    property on el. Otherwise, let result be the guaranteed-invalid value.
    let substituted_first = match substitute_tokens(store, registry, first_argument, context, recursion_depth + 1) {
        TokenResolution::Resolved(tokens) => tokens,
        TokenResolution::Invalid | TokenResolution::Cyclic => Vec::new(),
        TokenResolution::NotHandled => return TokenResolution::NotHandled,
    };
    let name = match trim_whitespace(&substituted_first) {
        [
            OwnedToken {
                kind: OwnedTokenKind::Ident(name),
                ..
            },
        ] if name.starts_with_ascii("--") => Some(name.as_slice()),
        _ => None,
    };
    if let Some(name) = name {
        match resolve_custom_property(store, registry, name, context, recursion_depth) {
            TokenResolution::Cyclic if comma.is_none() => return TokenResolution::Cyclic,
            TokenResolution::Invalid | TokenResolution::Cyclic => {}
            result => return result,
        }
    }
    let Some(comma) = comma else {
        return TokenResolution::Invalid;
    };
    // 4. If result contains the guaranteed-invalid value, and second arg was provided, set result to the result of
    //    substitute arbitrary substitution functions on second arg.
    substitute_tokens(store, registry, &arguments[comma + 1..], context, recursion_depth + 1)
}

fn replace_inherit_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // https://drafts.csswg.org/css-values-5/#replace-an-inherit-function
    let comma = find_top_level_comma(arguments);
    let first_argument = &arguments[..comma.unwrap_or(arguments.len())];
    let substituted_first = match substitute_tokens(store, registry, first_argument, context, recursion_depth + 1) {
        TokenResolution::Resolved(tokens) => tokens,
        TokenResolution::Invalid | TokenResolution::Cyclic => Vec::new(),
        TokenResolution::NotHandled => return TokenResolution::NotHandled,
    };
    let name = match trim_whitespace(&substituted_first) {
        [
            OwnedToken {
                kind: OwnedTokenKind::Ident(name),
                ..
            },
        ] if name.starts_with_ascii("--") => Some(name.as_slice()),
        _ => None,
    };
    if let Some(name) = name {
        let local_scope = context.function_local_scopes.pop();
        // FIXME: Inherited values should already be computed. This is not guaranteed during custom-function evaluation:
        //        the function's hypothetical element inherits from the calling element while that element's
        //        custom-property resolution batch can still be in progress. Until inherited lookup supplies a computed
        //        value, temporarily unguard the same property while resolving it from the selected store.
        let inherited_same_name = {
            let mut guarded_contexts = context.guarded_contexts.contexts.borrow_mut();
            guarded_contexts
                .iter()
                .rposition(|context| context.dependency.is_property(name, None))
                .map(|index| (index, guarded_contexts.remove(index)))
        };
        let inherited_store_override = context.inheritance_store_overrides.last().cloned().flatten();
        let inherited_store = if local_scope.is_some() {
            store
        } else {
            inherited_store_override.as_deref().or(context.inheritance_store)
        };
        let result = resolve_custom_property_with_lookup(
            inherited_store,
            registry,
            name,
            context,
            recursion_depth + 1,
            if local_scope.is_some() {
                CustomPropertyLookup::Normal
            } else {
                CustomPropertyLookup::ExplicitInheritance
            },
        );
        if let Some(local_scope) = local_scope {
            context.function_local_scopes.push(local_scope);
        }
        if let Some((index, inherited_same_name)) = inherited_same_name {
            context
                .guarded_contexts
                .contexts
                .borrow_mut()
                .insert(index, inherited_same_name);
        }
        match result {
            TokenResolution::Invalid | TokenResolution::Cyclic => {}
            result => return result,
        }
    }
    let Some(comma) = comma else {
        return TokenResolution::Invalid;
    };
    substitute_tokens(store, registry, &arguments[comma + 1..], context, recursion_depth + 1)
}

fn replace_env_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // https://drafts.csswg.org/css-env/#substitute-an-env
    let comma = find_top_level_comma(arguments);
    let first_argument = &arguments[..comma.unwrap_or(arguments.len())];
    let substituted_first = match substitute_tokens(store, registry, first_argument, context, recursion_depth + 1) {
        TokenResolution::Resolved(tokens) => tokens,
        TokenResolution::Invalid => return TokenResolution::Invalid,
        TokenResolution::Cyclic => return TokenResolution::Cyclic,
        TokenResolution::NotHandled => return TokenResolution::NotHandled,
    };
    let first_argument = trim_whitespace(&substituted_first);
    let Some(name) = (match first_argument.first() {
        Some(OwnedToken {
            kind: OwnedTokenKind::Ident(name),
            ..
        }) => Some(name.as_slice()),
        _ => None,
    }) else {
        return TokenResolution::Invalid;
    };
    let Some(indices) = (|| {
        let mut indices = Vec::new();
        for token in trim_whitespace(&first_argument[1..]) {
            if matches!(token.kind, OwnedTokenKind::Whitespace) {
                continue;
            }
            if !matches!(token.kind, OwnedTokenKind::Number) {
                return None;
            }
            let source = token.source.to_vec();
            let index = source.iter().try_fold(0i32, |value, &unit| {
                let digit = unit.checked_sub(u16::from(b'0'))?;
                (digit <= 9).then_some(value.checked_mul(10)?.checked_add(i32::from(digit))?)
            })?;
            if index < 0 {
                return None;
            }
            indices.push(index);
        }
        Some(indices)
    })() else {
        return TokenResolution::Invalid;
    };

    let variable = ENVIRONMENT_VARIABLES
        .iter()
        .find(|(known_name, _, _)| name.eq_ignore_ascii_case(known_name));
    if let Some((_, dimension_count, value_type)) = variable
        && *dimension_count == indices.len()
    {
        if *value_type == "<number>" {
            return TokenResolution::Resolved(tokenize_owned(b"1"));
        }
        if *dimension_count == 0 {
            return TokenResolution::Resolved(tokenize_owned(b"0px"));
        }
        // The C++ document oracle currently exposes no viewport segments, so every
        // recognized two-dimensional viewport-segment lookup is guaranteed-invalid.
        return TokenResolution::Invalid;
    }
    let Some(comma) = comma else {
        return TokenResolution::Invalid;
    };
    substitute_tokens(store, registry, &arguments[comma + 1..], context, recursion_depth + 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConditionEvaluation {
    Match(bool),
    Invalid,
    Cyclic,
    NotHandled,
}

#[derive(Debug)]
enum ParsedBooleanExpression {
    Test(Vec<OwnedToken>),
    Not(Box<ParsedBooleanExpression>),
    And(Vec<ParsedBooleanExpression>),
    Or(Vec<ParsedBooleanExpression>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConditionValidation {
    Valid,
    Invalid,
}

fn parse_boolean_expression(
    tokens: &[OwnedToken],
    validate_test: &mut impl FnMut(&[OwnedToken]) -> ConditionValidation,
) -> Result<ParsedBooleanExpression, ConditionValidation> {
    let tokens = trim_whitespace(tokens);
    if tokens.is_empty() {
        return Err(ConditionValidation::Invalid);
    }
    match validate_test(tokens) {
        ConditionValidation::Valid => return Ok(ParsedBooleanExpression::Test(tokens.to_vec())),
        ConditionValidation::Invalid => {}
    }
    if matches!(
        tokens.first(),
        Some(OwnedToken {
            kind: OwnedTokenKind::Ident(keyword),
            ..
        }) if keyword.eq_ignore_ascii_case("not")
    ) {
        return Ok(ParsedBooleanExpression::Not(Box::new(parse_boolean_group(
            &tokens[1..],
            validate_test,
        )?)));
    }

    let mut combinator = None;
    let mut groups = Vec::new();
    let mut group_start = 0;
    let mut index = 0;
    while index < tokens.len() {
        if matching_close(&tokens[index].kind).is_some() {
            let Some(close) = find_matching_close(tokens, index) else {
                return Err(ConditionValidation::Invalid);
            };
            index = close + 1;
            continue;
        }
        let current = match &tokens[index].kind {
            OwnedTokenKind::Ident(keyword) if keyword.eq_ignore_ascii_case("and") => Some(true),
            OwnedTokenKind::Ident(keyword) if keyword.eq_ignore_ascii_case("or") => Some(false),
            _ => None,
        };
        if let Some(current) = current {
            if combinator.is_some_and(|combinator| combinator != current) {
                return Err(ConditionValidation::Invalid);
            }
            combinator = Some(current);
            groups.push(parse_boolean_group(&tokens[group_start..index], validate_test)?);
            group_start = index + 1;
        }
        index += 1;
    }
    let Some(combinator) = combinator else {
        return parse_boolean_group(tokens, validate_test);
    };
    groups.push(parse_boolean_group(&tokens[group_start..], validate_test)?);
    Ok(if combinator {
        ParsedBooleanExpression::And(groups)
    } else {
        ParsedBooleanExpression::Or(groups)
    })
}

fn parse_boolean_group(
    tokens: &[OwnedToken],
    validate_test: &mut impl FnMut(&[OwnedToken]) -> ConditionValidation,
) -> Result<ParsedBooleanExpression, ConditionValidation> {
    let tokens = trim_whitespace(tokens);
    if matches!(tokens.first().map(|token| &token.kind), Some(OwnedTokenKind::OpenParen))
        && find_matching_close(tokens, 0) == Some(tokens.len() - 1)
    {
        return parse_boolean_expression(&tokens[1..tokens.len() - 1], validate_test);
    }
    match validate_test(tokens) {
        ConditionValidation::Valid => Ok(ParsedBooleanExpression::Test(tokens.to_vec())),
        validation => Err(validation),
    }
}

fn evaluate_parsed_boolean_expression(
    expression: &ParsedBooleanExpression,
    evaluate_test: &mut impl FnMut(&[OwnedToken]) -> ConditionEvaluation,
) -> ConditionEvaluation {
    match expression {
        ParsedBooleanExpression::Test(tokens) => evaluate_test(tokens),
        ParsedBooleanExpression::Not(child) => match evaluate_parsed_boolean_expression(child, evaluate_test) {
            ConditionEvaluation::Match(value) => ConditionEvaluation::Match(!value),
            other => other,
        },
        ParsedBooleanExpression::And(children) => {
            for child in children {
                match evaluate_parsed_boolean_expression(child, evaluate_test) {
                    ConditionEvaluation::Match(true) => {}
                    ConditionEvaluation::Match(false) => return ConditionEvaluation::Match(false),
                    other => return other,
                }
            }
            ConditionEvaluation::Match(true)
        }
        ParsedBooleanExpression::Or(children) => {
            for child in children {
                match evaluate_parsed_boolean_expression(child, evaluate_test) {
                    ConditionEvaluation::Match(false) => {}
                    ConditionEvaluation::Match(true) => return ConditionEvaluation::Match(true),
                    other => return other,
                }
            }
            ConditionEvaluation::Match(false)
        }
    }
}

fn registered_style_query_values_are_equal(
    registry: &CustomPropertyRegistry,
    syntax: &SyntaxNode,
    computed_tokens: &[OwnedToken],
    query_tokens: &[OwnedToken],
    length_resolution_context: Option<&crate::css::style_compute::FfiLengthResolutionContext>,
    tree_counting: Option<(u64, u64)>,
    color_resolution_input: Option<crate::css::color_resolution::ColorResolutionInput<'_>>,
) -> bool {
    let mut random_function_index = 0;
    let context = registry.parse_context(&mut random_function_index);
    let Some(computed) = parse_with_syntax(&context, &serialize_tokens(computed_tokens), syntax) else {
        return false;
    };
    let Some(query) = parse_with_syntax(&context, &serialize_tokens(query_tokens), syntax) else {
        return false;
    };
    let color_resolution_input = color_resolution_input
        .as_ref()
        .unwrap_or(&crate::css::color_resolution::EMPTY_INPUT);
    let computed_color = crate::css::color_resolution::to_color(&computed, color_resolution_input);
    let query_color = crate::css::color_resolution::to_color(&query, color_resolution_input);
    if computed_color.is_some() || query_color.is_some() {
        return computed_color.is_some() && computed_color == query_color;
    }
    if let (Some(computed), Some(query)) = (
        style_value_range_comparable(&computed, length_resolution_context, tree_counting),
        style_value_range_comparable(&query, length_resolution_context, tree_counting),
    ) {
        return computed.kind == query.kind && computed.value == query.value;
    }
    computed == query
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleRangeComparison {
    Equal,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleRangeNumericType {
    Number,
    Percentage,
    Length,
    Angle,
    Time,
    Frequency,
    Resolution,
}

#[derive(Clone, Copy)]
struct StyleRangeComparableValue {
    kind: StyleRangeNumericType,
    value: f64,
}

fn style_value_range_comparable(
    parsed: &StyleValueData,
    length_resolution_context: Option<&crate::css::style_compute::FfiLengthResolutionContext>,
    tree_counting: Option<(u64, u64)>,
) -> Option<StyleRangeComparableValue> {
    if matches!(parsed, StyleValueData::Calculated { .. })
        && let Some(length_resolution_context) = length_resolution_context
        && let Some(crate::css::calc::AbsolutizedCalculation::Value(value)) =
            crate::css::calc::absolutize_calculation_value(
                parsed,
                std::ptr::from_ref(length_resolution_context).cast(),
                tree_counting,
                &[],
            )
    {
        return style_value_range_comparable(&value, Some(length_resolution_context), None);
    }
    let length_resolution = crate::css::calc::LengthResolution {
        context: length_resolution_context,
        fallback: None,
    };
    let comparable = match parsed {
        StyleValueData::Number { value } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Number,
            value: *value,
        },
        StyleValueData::Integer { value } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Number,
            value: f64::from(*value),
        },
        StyleValueData::Length { value, unit } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Length,
            value: crate::css::calc::CalcNumericValue::Length {
                value: *value,
                unit: *unit,
            }
            .to_canonical_number(length_resolution),
        },
        StyleValueData::Percentage { value } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Percentage,
            value: *value,
        },
        StyleValueData::Angle { value, unit } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Angle,
            value: crate::css::calc::CalcNumericValue::Angle {
                value: *value,
                unit: *unit,
            }
            .to_canonical_number(length_resolution),
        },
        StyleValueData::Time { value, unit } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Time,
            value: crate::css::calc::CalcNumericValue::Time {
                value: *value,
                unit: *unit,
            }
            .to_canonical_number(length_resolution),
        },
        StyleValueData::Frequency { value, unit } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Frequency,
            value: crate::css::calc::CalcNumericValue::Frequency {
                value: *value,
                unit: *unit,
            }
            .to_canonical_number(length_resolution),
        },
        StyleValueData::Resolution { value, unit } => StyleRangeComparableValue {
            kind: StyleRangeNumericType::Resolution,
            value: crate::css::calc::CalcNumericValue::Resolution {
                value: *value,
                unit: *unit,
            }
            .to_canonical_number(length_resolution),
        },
        calculated @ StyleValueData::Calculated { .. } => {
            let (kind, value) =
                crate::css::calc::resolve_calculated_style_range_value(calculated, length_resolution_context)?;
            StyleRangeComparableValue {
                kind: match kind {
                    0 => StyleRangeNumericType::Number,
                    1 => StyleRangeNumericType::Percentage,
                    2 => StyleRangeNumericType::Length,
                    3 => StyleRangeNumericType::Angle,
                    4 => StyleRangeNumericType::Time,
                    5 => StyleRangeNumericType::Frequency,
                    6 => StyleRangeNumericType::Resolution,
                    _ => return None,
                },
                value,
            }
        }
        _ => return None,
    };
    comparable.value.is_finite().then_some(comparable)
}

fn style_range_comparisons(tokens: &[OwnedToken]) -> Option<Vec<(usize, usize, StyleRangeComparison)>> {
    let mut comparisons = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if matching_close(&tokens[index].kind).is_some() {
            index = find_matching_close(tokens, index)? + 1;
            continue;
        }
        let comparison = if tokens[index].source.equals_ascii(b"=") {
            Some((1, StyleRangeComparison::Equal))
        } else if tokens[index].source.equals_ascii(b"<") {
            if tokens
                .get(index + 1)
                .is_some_and(|token| token.source.equals_ascii(b"="))
            {
                Some((2, StyleRangeComparison::LessThanOrEqual))
            } else {
                Some((1, StyleRangeComparison::LessThan))
            }
        } else if tokens[index].source.equals_ascii(b">") {
            if tokens
                .get(index + 1)
                .is_some_and(|token| token.source.equals_ascii(b"="))
            {
                Some((2, StyleRangeComparison::GreaterThanOrEqual))
            } else {
                Some((1, StyleRangeComparison::GreaterThan))
            }
        } else {
            None
        };
        if let Some((length, comparison)) = comparison {
            comparisons.push((index, length, comparison));
            index += length;
        } else {
            index += 1;
        }
    }
    (comparisons.len() <= 2).then_some(comparisons)
}

fn style_range_comparable_value(
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &ASFResolutionContext,
) -> Option<StyleRangeComparableValue> {
    let source = serialize_tokens(tokens);
    let mut random_function_index = 0;
    let owned_parse_context;
    let parse_context = if let Some(parse_context) = context.parse_context {
        parse_context
    } else {
        owned_parse_context = registry?.parse_context(&mut random_function_index);
        &owned_parse_context
    };
    let syntax_types = [
        SyntaxType::Number,
        SyntaxType::Length,
        SyntaxType::Percentage,
        SyntaxType::Angle,
        SyntaxType::Time,
        SyntaxType::Frequency,
        SyntaxType::Resolution,
    ];
    let parsed = syntax_types
        .into_iter()
        .find_map(|syntax_type| parse_with_syntax(parse_context, &source, &SyntaxNode::Type(syntax_type)))?;
    style_value_range_comparable(
        &parsed,
        context.style_query_length_resolution_context,
        context.style_query_tree_counting,
    )
}

fn compare_style_range_values(
    left: StyleRangeComparableValue,
    comparison: StyleRangeComparison,
    right: StyleRangeComparableValue,
) -> bool {
    let dimension = |kind| !matches!(kind, StyleRangeNumericType::Number | StyleRangeNumericType::Percentage);
    if left.kind != right.kind
        && !(left.kind == StyleRangeNumericType::Number && left.value == 0.0 && dimension(right.kind))
        && !(right.kind == StyleRangeNumericType::Number && right.value == 0.0 && dimension(left.kind))
    {
        return false;
    }
    match comparison {
        StyleRangeComparison::Equal => left.value == right.value,
        StyleRangeComparison::LessThan => left.value < right.value,
        StyleRangeComparison::LessThanOrEqual => left.value <= right.value,
        StyleRangeComparison::GreaterThan => left.value > right.value,
        StyleRangeComparison::GreaterThanOrEqual => left.value >= right.value,
    }
}

fn evaluate_style_range_value(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> Result<Option<StyleRangeComparableValue>, ConditionEvaluation> {
    let tokens = trim_whitespace(tokens);
    let resolved = if let [
        OwnedToken {
            kind: OwnedTokenKind::Ident(name),
            ..
        },
    ] = tokens
        && name.starts_with_ascii("--")
    {
        if let Some(dependencies) = context.style_query_dependencies.as_deref_mut() {
            dependencies.note(name);
        }
        match resolve_custom_property(store, registry, name, context, recursion_depth + 1) {
            TokenResolution::Resolved(tokens) => tokens,
            TokenResolution::Invalid => return Ok(None),
            TokenResolution::Cyclic => return Err(ConditionEvaluation::Cyclic),
            TokenResolution::NotHandled => return Err(ConditionEvaluation::NotHandled),
        }
    } else {
        match substitute_arbitrary_substitution_functions(store, registry, tokens, context, recursion_depth + 1, None) {
            TokenResolution::Resolved(tokens) => tokens,
            TokenResolution::Invalid => return Ok(None),
            TokenResolution::Cyclic => return Err(ConditionEvaluation::Cyclic),
            TokenResolution::NotHandled => return Err(ConditionEvaluation::NotHandled),
        }
    };
    Ok(style_range_comparable_value(registry, &resolved, context))
}

fn evaluate_style_range(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    comparisons: &[(usize, usize, StyleRangeComparison)],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> ConditionEvaluation {
    if comparisons.is_empty() || comparisons.len() > 2 {
        return ConditionEvaluation::Invalid;
    }
    let (first_index, first_length, first_comparison) = comparisons[0];
    let middle_end = comparisons.get(1).map_or(tokens.len(), |comparison| comparison.0);
    let values = [
        &tokens[..first_index],
        &tokens[first_index + first_length..middle_end],
        comparisons
            .get(1)
            .map_or(&tokens[tokens.len()..], |(index, length, _)| &tokens[index + length..]),
    ];
    let left = match evaluate_style_range_value(store, registry, values[0], context, recursion_depth + 1) {
        Ok(Some(value)) => value,
        Ok(None) => return ConditionEvaluation::Match(false),
        Err(result) => return result,
    };
    let middle = match evaluate_style_range_value(store, registry, values[1], context, recursion_depth + 1) {
        Ok(Some(value)) => value,
        Ok(None) => return ConditionEvaluation::Match(false),
        Err(result) => return result,
    };
    if !compare_style_range_values(left, first_comparison, middle) {
        return ConditionEvaluation::Match(false);
    }
    let Some((_, _, second_comparison)) = comparisons.get(1).copied() else {
        return ConditionEvaluation::Match(true);
    };
    let right = match evaluate_style_range_value(store, registry, values[2], context, recursion_depth + 1) {
        Ok(Some(value)) => value,
        Ok(None) => return ConditionEvaluation::Match(false),
        Err(result) => return result,
    };
    ConditionEvaluation::Match(compare_style_range_values(middle, second_comparison, right))
}

fn validate_style_feature(tokens: &[OwnedToken]) -> ConditionValidation {
    let tokens = trim_whitespace(tokens);
    if find_top_level_source(tokens, b"!").is_some() {
        return ConditionValidation::Invalid;
    }
    if tokens.iter().any(|token| {
        token.source.equals_ascii(b"<") || token.source.equals_ascii(b">") || token.source.equals_ascii(b"=")
    }) {
        return ConditionValidation::Valid;
    }
    let colon = find_top_level_source(tokens, b":");
    let name_tokens = trim_whitespace(&tokens[..colon.unwrap_or(tokens.len())]);
    if !matches!(
        name_tokens,
        [OwnedToken {
            kind: OwnedTokenKind::Ident(_),
            ..
        }]
    ) {
        return ConditionValidation::Invalid;
    }
    if let Some(colon) = colon {
        let query = trim_whitespace(&tokens[colon + 1..]);
        if query.iter().any(|token| {
            token.source.equals_ascii(b"<") || token.source.equals_ascii(b">") || token.source.equals_ascii(b"=")
        }) {
            return ConditionValidation::Invalid;
        }
    }
    ConditionValidation::Valid
}

fn evaluate_style_feature(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> ConditionEvaluation {
    let tokens = trim_whitespace(tokens);
    let comparisons = style_range_comparisons(tokens);
    if comparisons.as_ref().is_some_and(|comparisons| !comparisons.is_empty()) {
        return evaluate_style_range(
            store,
            registry,
            tokens,
            comparisons.as_ref().expect("checked non-empty comparisons"),
            context,
            recursion_depth + 1,
        );
    }
    let colon = find_top_level_source(tokens, b":");
    let name_tokens = trim_whitespace(&tokens[..colon.unwrap_or(tokens.len())]);
    let [
        OwnedToken {
            kind: OwnedTokenKind::Ident(name),
            ..
        },
    ] = name_tokens
    else {
        return ConditionEvaluation::Invalid;
    };
    if !name.starts_with_ascii("--") {
        return ConditionEvaluation::Match(false);
    }

    let local_registration = context
        .function_local_scopes
        .last()
        .and_then(|scope| scope.registrations.get(name))
        .cloned();
    let registration = context
        .function_local_scopes
        .is_empty()
        .then(|| registry.and_then(|registry| registry.registrations.get(name)))
        .flatten();
    if let Some(dependencies) = context.style_query_dependencies.as_deref_mut() {
        dependencies.note(name);
    }
    let computed = resolve_custom_property(store, registry, name, context, recursion_depth + 1);
    let computed = match computed {
        TokenResolution::Resolved(tokens) => Some(tokens),
        TokenResolution::Invalid | TokenResolution::Cyclic => None,
        TokenResolution::NotHandled => return ConditionEvaluation::NotHandled,
    };
    let Some(colon) = colon else {
        return ConditionEvaluation::Match(computed.is_some());
    };
    let query = trim_whitespace(&tokens[colon + 1..]);

    if let Some(keyword) = single_css_wide_keyword(query) {
        if keyword.eq_ignore_ascii_case("revert") || keyword.eq_ignore_ascii_case("revert-layer") {
            return ConditionEvaluation::Match(false);
        }
        let expected = if keyword.eq_ignore_ascii_case("initial")
            || keyword.eq_ignore_ascii_case("unset") && registration.is_some_and(|registration| !registration.inherits)
        {
            local_registration
                .as_ref()
                .and_then(|registration| registration.initial_tokens.clone())
                .or_else(|| {
                    registration.and_then(|registration| registration.initial_source.as_ref().map(tokenize_owned))
                })
        } else {
            let local_scope = local_registration
                .as_ref()
                .and_then(|_| context.function_local_scopes.pop());
            let inherited_store = local_scope.as_ref().map_or(context.inheritance_store, |_| store);
            let result = resolve_custom_property(inherited_store, registry, name, context, recursion_depth + 1);
            if let Some(local_scope) = local_scope {
                context.function_local_scopes.push(local_scope);
            }
            match result {
                TokenResolution::Resolved(tokens) => Some(tokens),
                TokenResolution::Invalid => None,
                TokenResolution::Cyclic => return ConditionEvaluation::Cyclic,
                TokenResolution::NotHandled => return ConditionEvaluation::NotHandled,
            }
        };
        return ConditionEvaluation::Match(match (computed, expected) {
            (None, None) => true,
            (Some(computed), Some(expected)) if local_registration.is_some() || registration.is_some() => {
                registered_style_query_values_are_equal(
                    registry.expect("registered syntax requires registry"),
                    local_registration
                        .as_ref()
                        .map(|registration| &registration.syntax)
                        .unwrap_or_else(|| &registration.expect("registered property").syntax),
                    &computed,
                    &expected,
                    context.style_query_length_resolution_context,
                    context.style_query_tree_counting,
                    context.style_query_color_resolution_input,
                )
            }
            (Some(computed), Some(expected)) => trim_whitespace(&computed) == trim_whitespace(&expected),
            _ => false,
        });
    }

    let Some(computed) = computed else {
        return ConditionEvaluation::Match(false);
    };
    if let (Some(registry), Some(syntax)) = (
        registry,
        local_registration
            .as_ref()
            .map(|registration| &registration.syntax)
            .or_else(|| registration.map(|registration| &registration.syntax)),
    ) {
        return ConditionEvaluation::Match(registered_style_query_values_are_equal(
            registry,
            syntax,
            &computed,
            query,
            context.style_query_length_resolution_context,
            context.style_query_tree_counting,
            context.style_query_color_resolution_input,
        ));
    }
    ConditionEvaluation::Match(serialize_tokens(trim_whitespace(&computed)) == serialize_tokens(query))
}

pub(crate) unsafe fn evaluate_retained_container_style_feature(
    store: *const c_void,
    registry: &CustomPropertyRegistry,
    feature: crate::css::parser::query_parser::FfiContainerStyleFeature,
    length_resolution_context: &crate::css::style_compute::FfiLengthResolutionContext,
    dependencies: &mut crate::css::cascaded_properties::StyleQueryDependencies,
    tree_counting: (u64, u64),
    color_resolution_input: crate::css::color_resolution::ColorResolutionInput<'_>,
) -> crate::css::parser::query_parser::MatchResult {
    use crate::css::parser::query_parser::{FfiContainerStyleFeatureKind, FfiStyleRangeValueKind, MatchResult};
    let values = if feature.value_count == 0 {
        &[][..]
    } else {
        if feature.values.is_null() {
            return MatchResult::Unknown;
        }
        unsafe { std::slice::from_raw_parts(feature.values, feature.value_count) }
    };
    let mut source = Vec::new();
    let append_value = |source: &mut Vec<u16>, index: usize| -> bool {
        let Some(value) = values.get(index) else {
            return false;
        };
        let Some(units) = (unsafe { value.value.to_utf16() }) else {
            return false;
        };
        source.extend_from_slice(&units);
        true
    };
    let append_comparison = |source: &mut Vec<u16>, comparison: u8| -> bool {
        let text = match comparison {
            0 => "=",
            1 => "<",
            2 => "<=",
            3 => ">",
            4 => ">=",
            _ => return false,
        };
        source.extend(text.encode_utf16());
        true
    };
    let valid = match feature.kind {
        FfiContainerStyleFeatureKind::Boolean => append_value(&mut source, 0),
        FfiContainerStyleFeatureKind::Plain => {
            append_value(&mut source, 0) && {
                source.push(u16::from(b':'));
                append_value(&mut source, 1)
            }
        }
        FfiContainerStyleFeatureKind::Range => {
            append_value(&mut source, 0)
                && append_comparison(&mut source, feature.first_comparison)
                && append_value(&mut source, 1)
                && (values.len() == 2
                    || append_comparison(&mut source, feature.second_comparison) && append_value(&mut source, 2))
        }
    };
    if !valid
        || values.iter().any(|value| {
            !matches!(
                value.kind,
                FfiStyleRangeValueKind::Property | FfiStyleRangeValueKind::Components
            )
        })
    {
        return MatchResult::Unknown;
    }
    let store = unsafe { store.cast::<CustomPropertyStore>().as_ref() };
    let mut context = ASFResolutionContext {
        inheritance_store: store.and_then(|store| store.inheritance_parent.as_deref()),
        style_query_length_resolution_context: Some(length_resolution_context),
        style_query_dependencies: Some(dependencies),
        style_query_tree_counting: Some(tree_counting),
        style_query_color_resolution_input: Some(color_resolution_input),
        ..Default::default()
    };
    match evaluate_style_feature(store, Some(registry), &tokenize_owned(&source), &mut context, 0) {
        ConditionEvaluation::Match(true) => MatchResult::True,
        ConditionEvaluation::Match(false) => MatchResult::False,
        ConditionEvaluation::Invalid | ConditionEvaluation::NotHandled | ConditionEvaluation::Cyclic => {
            MatchResult::Unknown
        }
    }
}

fn evaluate_style_query(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> ConditionEvaluation {
    let expression = match parse_boolean_expression(tokens, &mut validate_style_feature) {
        Ok(expression) => expression,
        Err(ConditionValidation::Invalid) => return ConditionEvaluation::Invalid,
        Err(ConditionValidation::Valid) => unreachable!(),
    };
    evaluate_parsed_boolean_expression(&expression, &mut |feature| {
        evaluate_style_feature(store, registry, feature, context, recursion_depth + 1)
    })
}

fn validate_if_test(tokens: &[OwnedToken]) -> ConditionValidation {
    let test = trim_whitespace(tokens);
    let Some(OwnedTokenKind::Function(name)) = test.first().map(|token| &token.kind) else {
        return ConditionValidation::Invalid;
    };
    let Some(close) = find_matching_close(test, 0) else {
        return ConditionValidation::Invalid;
    };
    if close != test.len() - 1 {
        return ConditionValidation::Invalid;
    }
    if name.eq_ignore_ascii_case("style") {
        return match parse_boolean_expression(&test[1..close], &mut validate_style_feature) {
            Ok(_) => ConditionValidation::Valid,
            Err(validation) => validation,
        };
    }
    if name.eq_ignore_ascii_case("media") || name.eq_ignore_ascii_case("supports") {
        return ConditionValidation::Valid;
    }
    ConditionValidation::Valid
}

fn evaluate_if_condition(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> ConditionEvaluation {
    if matches!(
        trim_whitespace(tokens),
        [OwnedToken {
            kind: OwnedTokenKind::Ident(keyword),
            ..
        }] if keyword.eq_ignore_ascii_case("else")
    ) {
        return ConditionEvaluation::Match(true);
    }
    let expression = match parse_boolean_expression(tokens, &mut validate_if_test) {
        Ok(expression) => expression,
        Err(ConditionValidation::Invalid) => return ConditionEvaluation::Invalid,
        Err(ConditionValidation::Valid) => unreachable!(),
    };
    evaluate_parsed_boolean_expression(&expression, &mut |test| {
        let test = trim_whitespace(test);
        let OwnedTokenKind::Function(name) = &test[0].kind else {
            unreachable!("validated if condition test")
        };
        let close = test.len() - 1;
        if name.eq_ignore_ascii_case("style") {
            return evaluate_style_query(store, registry, &test[1..close], context, recursion_depth + 1);
        }
        if name.eq_ignore_ascii_case("media") {
            let Some(environment) = context.media_environment() else {
                return ConditionEvaluation::NotHandled;
            };
            let source = serialize_tokens(&test[1..close]);
            return match unsafe { parse_and_evaluate_media_if_condition(&source, environment) } {
                Some(MatchResult::True) => ConditionEvaluation::Match(true),
                Some(MatchResult::False | MatchResult::Unknown) => ConditionEvaluation::Match(false),
                None => ConditionEvaluation::Invalid,
            };
        }
        if name.eq_ignore_ascii_case("supports") {
            let Some(parse_context) = context.parse_context else {
                return ConditionEvaluation::NotHandled;
            };
            let source = serialize_tokens(&test[1..close]);
            return match unsafe { parse_and_evaluate_supports_if_condition(&source, parse_context) } {
                Some(MatchResult::True) => ConditionEvaluation::Match(true),
                Some(MatchResult::False | MatchResult::Unknown) => ConditionEvaluation::Match(false),
                None => ConditionEvaluation::Invalid,
            };
        }
        ConditionEvaluation::Match(false)
    })
}

fn split_function_arguments(tokens: &[OwnedToken]) -> Option<Vec<&[OwnedToken]>> {
    if trim_whitespace(tokens).is_empty() {
        return Some(Vec::new());
    }
    let mut arguments = Vec::new();
    let mut start = 0;
    loop {
        let remaining = &tokens[start..];
        let Some(comma) = find_top_level_comma(remaining) else {
            arguments.push(remaining);
            break;
        };
        arguments.push(&remaining[..comma]);
        start += comma + 1;
        if start > tokens.len() {
            return None;
        }
    }
    Some(arguments)
}

// https://drafts.csswg.org/css-mixins/#resolve-function-styles
fn resolve_function_styles<'a>(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    local_scope: FunctionLocalScope,
    property_names: impl IntoIterator<Item = &'a [u16]>,
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> HashMap<Vec<u16>, TokenResolution> {
    // 1. Create a "hypothetical element" el that acts as a child of calling context's element. el is featureless, and
    //    only custom properties and the result descriptor apply to it.

    // NB: FunctionLocalScope represents the custom properties and registrations of the hypothetical element. The rest
    //     of the calling context is represented by the stores and ASFResolutionContext.
    context.function_local_scopes.push(local_scope);

    // 2. Apply rule to el to the specified value stage, with the following changes:
    //
    // - Only the custom property registrations in registrations are visible; all other custom properties are treated as
    //   unregistered.
    //
    // FIXME: A registration miss in the current function can fall through to outer-function or document registrations.
    //        The specification says those registrations are treated as unregistered here.
    //
    // - The inherited value of calling context's property is the guaranteed-invalid value.
    //
    // FIXME: We do not represent the calling property's inherited value explicitly. Looking it up through normal
    //        context guarding can mark a cycle instead of producing the guaranteed-invalid value, which makes fallbacks
    //        behave differently from the specification.
    //
    // - On custom properties, initial resolves to the registration's initial value, inherit resolves like an inherit()
    //   function for that property, and any other CSS-wide keyword resolves to the guaranteed-invalid value. On result,
    //   CSS-wide keywords are left unresolved.
    //
    //   NB: resolve_function_local_property() implements the registration and CSS-wide keyword behavior.
    //
    // - During replacement of a custom property prop, the substitution context also includes custom function.
    //
    //   NB: resolve_function_local_property() includes the function identity in the property substitution context.

    // 3. Determine the computed value of all custom properties and the result "property" on el. Aside from custom
    //    property references and numbers/percentages, values that would normally refer to the element being styled refer
    //    to calling context's root element instead.
    let mut computed_values = HashMap::new();

    for name in property_names {
        let value = resolve_custom_property(store, registry, name, context, recursion_depth + 1);
        // FIXME: Duplicate parameter names make an @function rule invalid, but the parser currently accepts them. Keep
        //        the pre-refactor behavior where a later failed resolution does not replace an earlier successful one.
        //        See https://drafts.csswg.org/css-mixins/#function-prelude
        if matches!(&value, TokenResolution::Resolved(_))
            || !computed_values
                .get(name)
                .is_some_and(|value| matches!(value, TokenResolution::Resolved(_)))
        {
            computed_values.insert(name.to_vec(), value);
        }
    }

    context.function_local_scopes.pop().expect("function-local style scope");

    // 4. Return el's styles.
    computed_values
}

// https://drafts.csswg.org/css-mixins/#evaluate-a-custom-function
fn evaluate_a_custom_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    custom_function: &CustomFunctionDefinition,
    arguments: Vec<TokenResolution>,
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // 1. Let substitution context be a substitution context containing «"function", custom function».
    // Note: Due to tree-scoping, the same function name may appear multiple times on the stack while referring to
    //       different custom functions. For this reason, the custom function itself is included in the substitution
    //       context, not just its name.
    let substitution_context =
        SubstitutionContext::new(SubstitutionContextDependency::Function(custom_function.identity));

    // 2. Guard substitution context for the remainder of this algorithm. If substitution context is marked as cyclic,
    //    return the guaranteed-invalid value.
    let Some(_guard) = context.guarded_contexts.guard(&substitution_context) else {
        return TokenResolution::Cyclic;
    };

    // 3. If the number of items in arguments is greater than the number of function parameters in custom function,
    //    return the guaranteed-invalid value.
    if arguments.len() > custom_function.signature.parameters.len() {
        return TokenResolution::Invalid;
    }

    // 4. Let registrations be an initially empty set of custom property registrations.
    let mut registrations = Rc::new(HashMap::new());

    // 5. For each function parameter of custom function, create a custom property registration with the parameter's
    //    name, a syntax of the parameter type, an inherit flag of "true", and no initial value. Add the registration to
    //    registrations.
    for parameter in &custom_function.signature.parameters {
        Rc::get_mut(&mut registrations)
            .expect("Failed to get mutable reference to registrations")
            .insert(
                parameter.name.units().to_vec(),
                FunctionLocalRegistration {
                    syntax: (*parameter.syntax).clone(),
                    initial_tokens: None,
                    is_result: false,
                },
            );
    }

    // 6. If custom function has a return type, create a custom property registration with the name "result", a syntax
    //    of the return type, an inherit flag of "false", and no initial value. Add the registration to registrations.
    // NB: Custom functions always have a return type, defaulting to the universal syntax.
    let result_name: Vec<u16> = b"result".iter().copied().map(u16::from).collect();
    Rc::get_mut(&mut registrations)
        .expect("Failed to get mutable reference to registrations")
        .insert(
            result_name.clone(),
            FunctionLocalRegistration {
                syntax: (*custom_function.signature.return_type).clone(),
                initial_tokens: None,
                is_result: true,
            },
        );

    // NB: FunctionLocalRegistration does not store an inherit flag; resolve_function_local_property() encodes the
    //     parameter and result inheritance behavior.

    // 7. Let argument rule be an initially empty style rule.
    let mut argument_rule = HashMap::new();

    // 8. For each function parameter of custom function:
    for (index, parameter) in custom_function.signature.parameters.iter().enumerate() {
        // AD-HOC: Chrome (the only other implementer at time of writing) resolves the entire function to the
        //         guaranteed-invalid value if a parameter without a default value is omitted.
        //         See https://github.com/w3c/csswg-drafts/issues/14165
        if index >= arguments.len() && parameter.default_value.is_none() {
            return TokenResolution::Invalid;
        }

        // 1. Let arg value be the value of the corresponding argument in arguments, or the guaranteed-invalid value if
        //    there is no corresponding argument.
        let arg_value = arguments.get(index).unwrap_or(&TokenResolution::Invalid);

        // 2. Let default value be the parameter's default value.
        let default_value = custom_function.parameter_defaults[index].as_ref();

        // 3. Add a custom property to argument rule with a name of the parameter's name, and a value of
        //    'first-valid(arg value, default value)'.
        // FIXME: We haven't implemented first-valid() yet, so do the equivalent inline.
        let normalized_argument = match arg_value {
            TokenResolution::Resolved(tokens) => normalize_function_tokens(registry, &parameter.syntax, tokens),
            TokenResolution::NotHandled => return TokenResolution::NotHandled,
            TokenResolution::Invalid | TokenResolution::Cyclic => TokenResolution::Invalid,
        };

        let value = match normalized_argument {
            TokenResolution::Resolved(tokens) => Some(FunctionLocalValue {
                tokens,
                includes_substitution: false,
            }),
            TokenResolution::NotHandled => return TokenResolution::NotHandled,
            TokenResolution::Invalid | TokenResolution::Cyclic => default_value.map(|tokens| FunctionLocalValue {
                tokens: trim_whitespace(tokens).to_vec(),
                includes_substitution: true,
            }),
        };
        // NB: If neither value is valid, the missing declaration represents the guaranteed-invalid value: the
        //     parameter's registration has no initial value, so resolve_function_local_property() returns Invalid.
        if let Some(value) = value {
            argument_rule.insert(parameter.name.units().to_vec(), value);
        }
    }

    // 9. Resolve function styles using custom function, argument rule, registrations, and calling context. Let argument
    //    styles be the result.
    let mut argument_styles = resolve_function_styles(
        store,
        registry,
        FunctionLocalScope {
            function_identity: custom_function.identity,
            resolved_value_cache: Rc::new(RefCell::new(HashMap::new())),
            values: argument_rule,
            registrations: Rc::clone(&registrations),
        },
        custom_function
            .signature
            .parameters
            .iter()
            .map(|parameter| parameter.name.units()),
        context,
        recursion_depth,
    );

    // 10. Let body rule be the function body of custom function, as a style rule.
    let mut body_rule = HashMap::new();

    // 11. For each custom property registration of registrations except the registration with the name "result",
    //     set its initial value to the corresponding value in argument styles, and prepend a custom property to
    //     body rule with the property name and value in argument styles.
    let mutable_registrations = Rc::get_mut(&mut registrations)
        .expect("function-local registrations are uniquely owned after argument resolution clone is dropped");

    for (name, registration) in mutable_registrations.iter_mut() {
        if registration.is_result {
            continue;
        }

        let Some(TokenResolution::Resolved(tokens)) = argument_styles.remove(name) else {
            // NB: An absent value and no initial tokens represent the guaranteed-invalid value. Keeping the
            //     registration ensures the parameter still shadows values from the calling context.
            continue;
        };

        body_rule.insert(
            name.clone(),
            FunctionLocalValue {
                tokens: tokens.clone(),
                includes_substitution: false,
            },
        );

        registration.initial_tokens = Some(tokens);
    }

    // NB: Function declarations are inserted after the parameter values to emulate prepending those parameter values
    //     to the body rule.
    for (name, tokens, includes_substitution) in &custom_function.declarations {
        body_rule.insert(
            name.clone(),
            FunctionLocalValue {
                tokens: tokens.clone(),
                includes_substitution: *includes_substitution,
            },
        );
    }

    // 12. Resolve function styles using custom function, body rule, registrations, and calling context. Let body styles
    //     be the result.
    let mut body_styles = resolve_function_styles(
        store,
        registry,
        FunctionLocalScope {
            function_identity: custom_function.identity,
            resolved_value_cache: Rc::new(RefCell::new(HashMap::new())),
            values: body_rule,
            registrations,
        },
        custom_function
            .declarations
            .iter()
            .map(|(name, _, _)| name.as_slice())
            .filter(|name| *name != result_name.as_slice())
            .chain(std::iter::once(result_name.as_slice())),
        context,
        recursion_depth,
    );

    // 13. If substitution context is marked as a cyclic substitution context, return the guaranteed-invalid value.
    // Note: Nested arbitrary substitution functions may have marked substitution context as cyclic at some point after
    //       step 2, for example when resolving result.
    if substitution_context.is_cyclic.get() {
        return TokenResolution::Cyclic;
    }

    // 14. Return the value of the result property in body styles.
    body_styles.remove(&result_name).unwrap_or(TokenResolution::Invalid)
}

// https://drafts.csswg.org/css-mixins/#replace-a-dashed-function
fn replace_a_dashed_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    name: &[u16],
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // 1. Let function be the result of dereferencing the dashed function's name as a tree-scoped reference. If no such
    //    name exists, return the guaranteed-invalid value.
    let Some(functions) = context.custom_functions else {
        return TokenResolution::NotHandled;
    };

    // NB: Nested calls are resolved from the scope where the calling function was defined.
    // FIXME: Top-level calls use the element's style scope, but should use the relevant CSS rule's style scope. See the
    //        failing tests in function-shadow.html.
    let caller_scope_identity = context
        .guarded_contexts
        .innermost_function()
        .and_then(|identity| {
            functions
                .definitions
                .iter()
                .find(|definition| definition.identity == identity)
                .map(|definition| definition.scope_identity)
        })
        .unwrap_or(functions.caller_scope_identity);
    let resolved_identity = functions
        .visible_definitions
        .get(&(caller_scope_identity, name.to_vec()))
        .copied();
    let definition = resolved_identity.and_then(|identity| {
        functions
            .definitions
            .iter()
            .find(|definition| definition.identity == identity)
    });
    let Some(function) = definition else {
        return TokenResolution::Invalid;
    };

    // FIXME: The generic arbitrary-substitution algorithm should parse the argument grammar before invoking this
    //        replacement algorithm. The existing Rust path receives raw contents and parses them here instead.
    let Some(argument_slices) = split_function_arguments(arguments) else {
        return TokenResolution::Invalid;
    };

    // 2. For each arg in arguments, substitute arbitrary substitution functions in arg, and replace arg with the
    //    result.
    // Note: This may leave some (or all) arguments as the guaranteed-invalid value, triggering default values (if any).
    let mut substituted_arguments = Vec::with_capacity(argument_slices.len());
    for arg in argument_slices {
        let mut argument = trim_whitespace(arg);
        // NB: Braces disambiguate an argument that contains a top-level comma; they are not part of the argument value.
        if matches!(
            argument.first().map(|token| &token.kind),
            Some(OwnedTokenKind::OpenCurly)
        ) && find_matching_close(argument, 0) == Some(argument.len() - 1)
        {
            argument = &argument[1..argument.len() - 1];
        }

        substituted_arguments.push(substitute_tokens(
            store,
            registry,
            argument,
            context,
            recursion_depth + 1,
        ));
    }

    // 3. If dashed function is being substituted into a property on an element, let calling context be a calling
    //    context with that element and that property. Otherwise, let calling context contain the hypothetical element
    //    and descriptor into which the function is being substituted.

    // NB: The calling context is represented by store, registry, and ASFResolutionContext, passed below.

    // 4. Evaluate a custom function, using function, arguments, and calling context, and return the equivalent token
    //    sequence of the value resulting from the evaluation.
    evaluate_a_custom_function(
        store,
        registry,
        function,
        substituted_arguments,
        context,
        recursion_depth,
    )
}

fn replace_if_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // https://drafts.csswg.org/css-values-5/#replace-an-if-function
    let mut branch_start = 0;
    while branch_start < arguments.len() {
        let branch_length =
            find_top_level_source(&arguments[branch_start..], b";").unwrap_or(arguments.len() - branch_start);
        let branch_end = branch_start + branch_length;
        let branch = &arguments[branch_start..branch_end];
        let Some(colon) = find_top_level_source(branch, b":") else {
            return TokenResolution::Invalid;
        };
        let condition = match substitute_tokens(store, registry, &branch[..colon], context, recursion_depth + 1) {
            TokenResolution::Resolved(tokens) => tokens,
            TokenResolution::Invalid | TokenResolution::Cyclic => branch[..colon].to_vec(),
            TokenResolution::NotHandled => return TokenResolution::NotHandled,
        };
        match evaluate_if_condition(store, registry, &condition, context, recursion_depth + 1) {
            ConditionEvaluation::Match(true) => {
                return substitute_tokens(store, registry, &branch[colon + 1..], context, recursion_depth + 1);
            }
            ConditionEvaluation::Match(false) | ConditionEvaluation::Invalid => {}
            ConditionEvaluation::Cyclic => return TokenResolution::Cyclic,
            ConditionEvaluation::NotHandled => return TokenResolution::NotHandled,
        }
        branch_start = branch_end + 1;
    }
    TokenResolution::Resolved(Vec::new())
}

enum AttrSyntax {
    Omitted,
    RawString,
    Number,
    Unit(Vec<u16>),
    Syntax(SyntaxNode),
}

enum AttrSyntaxParseFailure {
    SyntaxOmitted,
    SyntaxSpecified,
}

fn parse_attr_syntax(tokens: &[OwnedToken]) -> Result<(Vec<u16>, AttrSyntax), AttrSyntaxParseFailure> {
    let tokens = trim_whitespace(tokens);
    let Some(first) = tokens.first() else {
        return Err(AttrSyntaxParseFailure::SyntaxOmitted);
    };
    let OwnedTokenKind::Ident(name) = &first.kind else {
        return Err(AttrSyntaxParseFailure::SyntaxOmitted);
    };
    let rest = trim_whitespace(&tokens[1..]);
    if rest.is_empty() {
        return Ok((name.clone(), AttrSyntax::Omitted));
    }
    if let OwnedTokenKind::Ident(syntax) = &rest[0].kind {
        let syntax = if syntax.eq_ignore_ascii_case("raw-string") {
            AttrSyntax::RawString
        } else if syntax.eq_ignore_ascii_case("number") {
            AttrSyntax::Number
        } else if crate::css::parser::value_parser::is_dimension_unit(syntax) {
            AttrSyntax::Unit(syntax.clone())
        } else {
            return Err(AttrSyntaxParseFailure::SyntaxOmitted);
        };
        if rest.len() != 1 {
            return Err(AttrSyntaxParseFailure::SyntaxSpecified);
        }
        return Ok((name.clone(), syntax));
    }
    if matches!(rest[0].kind, OwnedTokenKind::Delim(37)) {
        if rest.len() != 1 {
            return Err(AttrSyntaxParseFailure::SyntaxSpecified);
        }
        return Ok((name.clone(), AttrSyntax::Unit(vec![u16::from(b'%')])));
    }
    if matches!(&rest[0].kind, OwnedTokenKind::Function(function) if function.eq_ignore_ascii_case("type"))
        && let Some(close) = find_matching_close(rest, 0)
    {
        let Some(syntax) = parse_syntax(&serialize_tokens(&rest[1..close]), false) else {
            return Err(AttrSyntaxParseFailure::SyntaxOmitted);
        };
        if close != rest.len() - 1 {
            return Err(AttrSyntaxParseFailure::SyntaxSpecified);
        }
        return Ok((name.clone(), AttrSyntax::Syntax(syntax)));
    }
    Err(AttrSyntaxParseFailure::SyntaxOmitted)
}

fn parse_attr_value_with_syntax(
    registry: Option<&CustomPropertyRegistry>,
    source: &[u16],
    syntax: &SyntaxNode,
) -> Option<Vec<OwnedToken>> {
    let registry = registry?;
    let mut random_function_index = 0;
    let parsed = parse_with_syntax(&registry.parse_context(&mut random_function_index), source, syntax)?;
    let serialized = crate::css::serialize::serialize_style_value_to_utf16(&parsed)?;
    Some(tokenize_owned(&serialized))
}

fn attr_fallback(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    comma: Option<usize>,
    syntax_was_omitted: bool,
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    let Some(comma) = comma else {
        if syntax_was_omitted {
            return TokenResolution::Resolved(tokenize_owned(b"\"\""));
        }
        return TokenResolution::Invalid;
    };
    substitute_tokens(store, registry, &arguments[comma + 1..], context, recursion_depth + 1)
}

fn replace_attr_function(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    arguments: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    // https://drafts.csswg.org/css-values-5/#replace-an-attr-function
    let comma = find_top_level_comma(arguments);
    let first_argument = &arguments[..comma.unwrap_or(arguments.len())];
    let substituted_first = match substitute_tokens(store, registry, first_argument, context, recursion_depth + 1) {
        TokenResolution::Resolved(tokens) => tokens,
        TokenResolution::Invalid | TokenResolution::Cyclic => {
            return attr_fallback(store, registry, arguments, comma, true, context, recursion_depth);
        }
        TokenResolution::NotHandled => return TokenResolution::NotHandled,
    };
    let (attribute_name, syntax) = match parse_attr_syntax(&substituted_first) {
        Ok(parsed) => parsed,
        Err(failure) => {
            return attr_fallback(
                store,
                registry,
                arguments,
                comma,
                matches!(failure, AttrSyntaxParseFailure::SyntaxOmitted),
                context,
                recursion_depth,
            );
        }
    };
    let syntax_was_omitted = matches!(syntax, AttrSyntax::Omitted);
    let attribute_value = context.attributes.and_then(|attributes| {
        attributes.get(&attribute_name).or_else(|| {
            context.attribute_names_are_ascii_case_insensitive.then(|| {
                attributes
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case_utf16(&attribute_name))
                    .map(|(_, value)| value)
            })?
        })
    });
    let Some(attribute_value) = attribute_value else {
        return attr_fallback(
            store,
            registry,
            arguments,
            comma,
            syntax_was_omitted,
            context,
            recursion_depth,
        );
    };

    let resolved = match syntax {
        AttrSyntax::Omitted | AttrSyntax::RawString => tokenize_owned(
            crate::css::css_tokenizer::TokenizerInput::Utf16(&crate::css::serialize::serialize_string(attribute_value)),
        ),
        AttrSyntax::Number => {
            if !matches!(
                trim_whitespace(&tokenize_owned(attribute_value)),
                [OwnedToken {
                    kind: OwnedTokenKind::Number,
                    ..
                }]
            ) {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            }
            let syntax = SyntaxNode::Type(crate::css::parser::syntax::SyntaxType::Number);
            let Some(tokens) = parse_attr_value_with_syntax(registry, attribute_value, &syntax) else {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            };
            tokens
        }
        AttrSyntax::Unit(unit) => {
            if !matches!(
                trim_whitespace(&tokenize_owned(attribute_value)),
                [OwnedToken {
                    kind: OwnedTokenKind::Number,
                    ..
                }]
            ) {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            }
            let number_syntax = SyntaxNode::Type(crate::css::parser::syntax::SyntaxType::Number);
            let Some(number) = parse_attr_value_with_syntax(registry, attribute_value, &number_syntax) else {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            };
            let mut source = serialize_tokens(&number);
            source.extend_from_slice(&unit);
            tokenize_owned(crate::css::css_tokenizer::TokenizerInput::Utf16(&source))
        }
        AttrSyntax::Syntax(syntax) => {
            let substituted = match substitute_arbitrary_substitution_functions(
                store,
                registry,
                &tokenize_owned(attribute_value),
                context,
                recursion_depth + 1,
                Some(SubstitutionContextDependency::Attribute(attribute_name.clone())),
            ) {
                TokenResolution::Resolved(tokens) => Some(serialize_tokens(&tokens)),
                TokenResolution::Invalid | TokenResolution::Cyclic => None,
                TokenResolution::NotHandled => return TokenResolution::NotHandled,
            };
            let Some(substituted) = substituted else {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            };
            let Some(tokens) = parse_attr_value_with_syntax(registry, &substituted, &syntax) else {
                return attr_fallback(store, registry, arguments, comma, false, context, recursion_depth);
            };
            tokens
        }
    };
    context.contains_attr_tainted_values = true;
    TokenResolution::Resolved(resolved)
}

// Step 2 of https://drafts.csswg.org/css-values-5/#substitute-arbitrary-substitution-function
fn substitute_tokens(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
) -> TokenResolution {
    if recursion_depth > MAX_SUBSTITUTION_RECURSION_DEPTH {
        return TokenResolution::Invalid;
    }

    let mut output = Vec::new();
    let mut index = 0;
    let mut is_cyclic = false;
    while index < tokens.len() {
        let Some(close_index) = matching_close(&tokens[index].kind).and_then(|_| find_matching_close(tokens, index))
        else {
            output.push(tokens[index].clone());
            index += 1;
            continue;
        };

        let contents = &tokens[index + 1..close_index];
        let resolved = match &tokens[index].kind {
            OwnedTokenKind::Function(name) if name.eq_ignore_ascii_case("var") => {
                replace_var_function(store, registry, contents, context, recursion_depth)
            }
            OwnedTokenKind::Function(name) if name.eq_ignore_ascii_case("attr") => {
                replace_attr_function(store, registry, contents, context, recursion_depth)
            }
            OwnedTokenKind::Function(name) if name.eq_ignore_ascii_case("inherit") => {
                replace_inherit_function(store, registry, contents, context, recursion_depth)
            }
            OwnedTokenKind::Function(name) if name.eq_ignore_ascii_case("env") => {
                replace_env_function(store, registry, contents, context, recursion_depth)
            }
            OwnedTokenKind::Function(name) if name.eq_ignore_ascii_case("if") => {
                replace_if_function(store, registry, contents, context, recursion_depth)
            }
            OwnedTokenKind::Function(name) if name.starts_with_ascii("--") => {
                replace_a_dashed_function(store, registry, name, contents, context, recursion_depth)
            }
            _ => substitute_tokens(store, registry, contents, context, recursion_depth + 1),
        };
        let resolved = match resolved {
            TokenResolution::Resolved(resolved) => resolved,
            TokenResolution::Invalid => return TokenResolution::Invalid,
            TokenResolution::Cyclic => {
                is_cyclic = true;
                index = close_index + 1;
                continue;
            }
            TokenResolution::NotHandled => return TokenResolution::NotHandled,
        };

        if !matches!(tokens[index].kind, OwnedTokenKind::Function(ref name) if name.starts_with_ascii("--") || name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("attr") || name.eq_ignore_ascii_case("env") || name.eq_ignore_ascii_case("if") || name.eq_ignore_ascii_case("inherit"))
        {
            output.push(tokens[index].clone());
        }
        let resolved_start = usize::from(
            matches!(output.last().map(|token| &token.kind), Some(OwnedTokenKind::Whitespace))
                && matches!(
                    resolved.first().map(|token| &token.kind),
                    Some(OwnedTokenKind::Whitespace)
                ),
        );
        output.extend(resolved.into_iter().skip(resolved_start));
        if !matches!(tokens[index].kind, OwnedTokenKind::Function(ref name) if name.starts_with_ascii("--") || name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("attr") || name.eq_ignore_ascii_case("env") || name.eq_ignore_ascii_case("if") || name.eq_ignore_ascii_case("inherit"))
        {
            output.push(tokens[close_index].clone());
        }
        let remaining_token_count = tokens.len() - close_index - 1;
        if output.len() + remaining_token_count > MAX_SUBSTITUTED_TOKEN_COUNT {
            return TokenResolution::Invalid;
        }
        index = close_index + 1;
    }
    if is_cyclic {
        TokenResolution::Cyclic
    } else {
        TokenResolution::Resolved(output)
    }
}

// https://drafts.csswg.org/css-values-5/#substitute-arbitrary-substitution-function
fn substitute_arbitrary_substitution_functions(
    store: Option<&CustomPropertyStore>,
    registry: Option<&CustomPropertyRegistry>,
    tokens: &[OwnedToken],
    context: &mut ASFResolutionContext,
    recursion_depth: u32,
    substitution_context: Option<SubstitutionContextDependency>,
) -> TokenResolution {
    let Some(dependency) = substitution_context else {
        return substitute_tokens(store, registry, tokens, context, recursion_depth);
    };

    // 1. Guard context for the remainder of this algorithm. If context is marked as a cyclic substitution context,
    //    return the guaranteed-invalid value.
    let substitution_context = SubstitutionContext::new(dependency);
    let Some(_guard) = context.guarded_contexts.guard(&substitution_context) else {
        return TokenResolution::Cyclic;
    };

    // 2. Substitute each arbitrary substitution function in values.
    let result = substitute_tokens(store, registry, tokens, context, recursion_depth);

    // 3. If context is marked as a cyclic substitution context, return the guaranteed-invalid value.
    // NOTE: Nested arbitrary substitution functions may have marked context as cyclic in step 2.
    if substitution_context.is_cyclic.get() {
        TokenResolution::Cyclic
    } else {
        result
    }
}

// https://drafts.csswg.org/css-syntax/#serialization
fn needs_comment_between(first: &OwnedTokenKind, second: &OwnedTokenKind) -> bool {
    let second_is_common = matches!(
        second,
        OwnedTokenKind::Url
            | OwnedTokenKind::BadUrl
            | OwnedTokenKind::Number
            | OwnedTokenKind::Percentage
            | OwnedTokenKind::Dimension
            | OwnedTokenKind::Cdc
    );
    let second_is_ident = matches!(second, OwnedTokenKind::Ident(_));
    let second_is_function = matches!(second, OwnedTokenKind::Function(_));
    let common = second_is_common || second_is_ident;

    match first {
        OwnedTokenKind::Ident(_) => {
            second_is_function || matches!(second, OwnedTokenKind::OpenParen | OwnedTokenKind::Delim(45)) || common
        }
        OwnedTokenKind::AtKeyword
        | OwnedTokenKind::Hash
        | OwnedTokenKind::Dimension
        | OwnedTokenKind::Delim(35 | 45) => second_is_function || matches!(second, OwnedTokenKind::Delim(45)) || common,
        OwnedTokenKind::Number => second_is_function || matches!(second, OwnedTokenKind::Delim(37)) || common,
        OwnedTokenKind::Delim(64) => {
            second_is_function
                || matches!(second, OwnedTokenKind::Delim(45))
                || second_is_ident
                || matches!(
                    second,
                    OwnedTokenKind::Url | OwnedTokenKind::BadUrl | OwnedTokenKind::Cdc
                )
        }
        OwnedTokenKind::Delim(46 | 43) => matches!(
            second,
            OwnedTokenKind::Number | OwnedTokenKind::Percentage | OwnedTokenKind::Dimension
        ),
        OwnedTokenKind::Delim(47) => matches!(second, OwnedTokenKind::Delim(42)),
        _ => false,
    }
}

fn serialize_tokens(tokens: &[OwnedToken]) -> Vec<u16> {
    let mut output = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        token.source.append_to(&mut output);
        if let Some(next) = tokens.get(index + 1)
            && needs_comment_between(&token.kind, &next.kind)
        {
            output.extend(b"/**/".iter().copied().map(u16::from));
        }
    }
    output
}

#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn resolve_vars(
    store: *const c_void,
    inheritance_store: *const c_void,
    registry: *const c_void,
    parse_context: Option<&ParseContext>,
    media_environment: Option<&FfiMediaEnvironment>,
    property_id: u16,
    root_custom_property_name: FfiUtf16View,
    value_data: *const c_void,
    environment: &mut VarResolutionEnvironment,
    attribute_names_are_ascii_case_insensitive: bool,
    style_query_length_resolution_context: *const crate::css::style_compute::FfiLengthResolutionContext,
    style_query_dependencies: *mut c_void,
    final_custom_properties: Option<&HashMap<Vec<u16>, *const c_void>>,
) -> NativeVarResolution {
    let store = if store.is_null() {
        None
    } else {
        Some(unsafe { &*store.cast::<CustomPropertyStore>() })
    };
    let registry = if registry.is_null() {
        None
    } else {
        Some(unsafe { &*registry.cast::<CustomPropertyRegistry>() })
    };
    let inheritance_store = if inheritance_store.is_null() {
        None
    } else {
        Some(unsafe { &*inheritance_store.cast::<CustomPropertyStore>() })
    };
    let value_data = unsafe { &*value_data.cast::<StyleValueData>() };
    let root_custom_property_name = unsafe { root_custom_property_name.to_utf16() }.filter(|name| !name.is_empty());
    assert_eq!(
        root_custom_property_name.is_some(),
        property_id == crate::css::property_metadata::property_id::CUSTOM,
        "substitution root must be either a named custom property or a non-custom property",
    );
    let root_property_name = root_custom_property_name.unwrap_or_else(|| {
        crate::css::property_metadata::property_name(property_id)
            .encode_utf16()
            .collect()
    });
    let substitution_context = Some(SubstitutionContextDependency::Property(root_property_name, None));
    let VarResolutionEnvironment {
        attributes,
        custom_functions,
        token_cache,
        resolution_stats,
    } = environment;
    let mut context = ASFResolutionContext {
        attributes: Some(attributes),
        inheritance_store,
        attribute_names_are_ascii_case_insensitive,
        contains_attr_tainted_values: false,
        custom_functions: Some(custom_functions),
        parse_context,
        media_environment,
        style_query_length_resolution_context: unsafe { style_query_length_resolution_context.as_ref() },
        style_query_dependencies: unsafe {
            style_query_dependencies
                .cast::<crate::css::cascaded_properties::StyleQueryDependencies>()
                .as_mut()
        },
        final_custom_properties,
        token_cache: Some(token_cache),
        resolution_stats: Some(resolution_stats),
        ..Default::default()
    };
    let Some((source, includes_substitution, contains_attr_tainted_values)) =
        cached_tokens_for_custom_property_value(value_data, &mut context)
    else {
        return NativeVarResolution::NotHandled;
    };
    // A value holding no substitution function has nothing to resolve: it is invalid at
    // computed-value time, as every caller takes an unresolved value it cannot substitute.
    if !includes_substitution {
        return NativeVarResolution::Invalid;
    }
    context.contains_attr_tainted_values = contains_attr_tainted_values;
    let result =
        substitute_arbitrary_substitution_functions(store, registry, &source, &mut context, 0, substitution_context);
    match result {
        TokenResolution::Resolved(tokens) => NativeVarResolution::Resolved {
            source: serialize_tokens(&tokens),
            contains_attr_tainted_values: context.contains_attr_tainted_values,
        },
        TokenResolution::Invalid | TokenResolution::Cyclic => NativeVarResolution::Invalid,
        TokenResolution::NotHandled => NativeVarResolution::NotHandled,
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn custom_property_values_outlive_their_store() {
        let name = utf16("--value");
        let store = CustomPropertyStore::child(
            None,
            vec![(
                0,
                CustomPropertyEntry {
                    _name: RetainedUtf16FlyString::none(),
                    name: name.into(),
                    value: RetainedStyleValueData::from_owned(StyleValueData::Number { value: 1.0 }),
                    important: false,
                },
            )],
        );
        // SAFETY: child returns one owned Arc reference.
        let store = unsafe { Arc::from_raw(store.cast::<CustomPropertyStore>()) };
        let value = store.get(0).unwrap().value.clone();
        drop(store);
        assert!(matches!(value.data(), StyleValueData::Number { value } if *value == 1.0));
    }

    fn utf16(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }

    fn substitute_without_custom_properties(source: &str) -> TokenResolution {
        substitute_tokens(
            None,
            None,
            &tokenize_owned(source.as_bytes()),
            &mut ASFResolutionContext::default(),
            0,
        )
    }

    fn substitute_with_attributes(source: &str, attributes: &[(&str, &str)]) -> (TokenResolution, bool) {
        let attributes: HashMap<_, _> = attributes
            .iter()
            .map(|(name, value)| (utf16(name), utf16(value)))
            .collect();
        let mut context = ASFResolutionContext {
            attributes: Some(&attributes),
            ..Default::default()
        };
        let result = substitute_tokens(None, None, &tokenize_owned(source.as_bytes()), &mut context, 0);
        (result, context.contains_attr_tainted_values)
    }

    #[test]
    fn missing_variable_uses_fallback() {
        let TokenResolution::Resolved(tokens) = substitute_without_custom_properties("calc(var(--missing, 1px) + 2px)")
        else {
            panic!("expected fallback substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("calc( 1px + 2px)"));
    }

    #[test]
    fn empty_fallback_is_valid() {
        let TokenResolution::Resolved(tokens) = substitute_without_custom_properties("var(--missing,)") else {
            panic!("expected empty fallback substitution");
        };
        assert!(tokens.is_empty());
    }

    #[test]
    fn missing_variable_without_fallback_is_invalid() {
        assert!(matches!(
            substitute_without_custom_properties("var(--missing)"),
            TokenResolution::Invalid
        ));
    }

    #[test]
    fn root_custom_property_cycles_do_not_take_var_fallbacks() {
        let mut context = ASFResolutionContext::default();
        assert!(matches!(
            substitute_arbitrary_substitution_functions(
                None,
                None,
                &tokenize_owned(b"var(--root, fallback)"),
                &mut context,
                0,
                Some(SubstitutionContextDependency::Property(utf16("--root"), None)),
            ),
            TokenResolution::Cyclic
        ));
    }

    #[test]
    fn substitution_context_cycle_marks_the_entire_mixed_context_suffix() {
        let guarded_contexts = GuardedSubstitutionContexts::default();
        let guard = |dependency| {
            let context = SubstitutionContext::new(dependency);
            let guard = guarded_contexts.guard(&context).unwrap();
            (context, guard)
        };
        let (root_context, _root_guard) = guard(SubstitutionContextDependency::Property(utf16("--root"), None));
        let (attribute_context, _attribute_guard) =
            guard(SubstitutionContextDependency::Attribute(utf16("data-value")));
        let (function_context, _function_guard) = guard(SubstitutionContextDependency::Function(1));

        let duplicate = SubstitutionContext::new(SubstitutionContextDependency::Attribute(utf16("data-value")));
        assert!(guarded_contexts.guard(&duplicate).is_none());
        assert!(!root_context.is_cyclic.get());
        assert!(attribute_context.is_cyclic.get());
        assert!(function_context.is_cyclic.get());
        assert!(duplicate.is_cyclic.get());
    }

    #[test]
    fn substitutes_environment_values_and_fallbacks() {
        let TokenResolution::Resolved(tokens) = substitute_without_custom_properties("env(safe-area-inset-top)") else {
            panic!("expected safe-area environment substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("0px"));

        let TokenResolution::Resolved(tokens) =
            substitute_without_custom_properties("env(unknown-environment-variable, 4px)")
        else {
            panic!("expected environment fallback");
        };
        assert_eq!(serialize_tokens(&tokens), utf16(" 4px"));

        assert!(matches!(
            substitute_without_custom_properties("env(\"unknown-environment-variable\", 4px)"),
            TokenResolution::Invalid
        ));
        assert!(matches!(
            substitute_without_custom_properties("env(safe-area-inset-top 1.5, 4px)"),
            TokenResolution::Invalid
        ));
    }

    #[test]
    fn substitutes_unconditional_if_branches() {
        let TokenResolution::Resolved(tokens) = substitute_without_custom_properties("if(else: 5px)") else {
            panic!("expected unconditional branch substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16(" 5px"));
    }

    #[test]
    fn substitutes_custom_functions_from_a_snapshot() {
        let functions = CustomFunctionRegistry {
            caller_scope_identity: 1,
            definitions: vec![CustomFunctionDefinition {
                identity: 2,
                scope_identity: 1,
                signature: Arc::new(FunctionSignature {
                    name: crate::css::css_string::CssString::from_utf16(&utf16("--echo")),
                    parameters: vec![crate::css::function_signature::FunctionParameterData {
                        name: crate::css::css_string::CssString::from_utf16(&utf16("--value")),
                        syntax: Arc::new(SyntaxNode::Universal),
                        default_value: None,
                    }]
                    .into_boxed_slice(),
                    return_type: Arc::new(SyntaxNode::Universal),
                }),
                parameter_defaults: vec![None],
                declarations: vec![(utf16("result"), tokenize_owned(b"var(--value)"), true)],
            }],
            visible_definitions: HashMap::from([((1, utf16("--echo")), 2)]),
        };
        let mut context = ASFResolutionContext {
            custom_functions: Some(&functions),
            ..Default::default()
        };
        let TokenResolution::Resolved(tokens) =
            substitute_tokens(None, None, &tokenize_owned(b"--echo(12px)"), &mut context, 0)
        else {
            panic!("expected custom function substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("12px"));
    }

    #[test]
    fn serialization_preserves_token_boundaries() {
        let TokenResolution::Resolved(tokens) = substitute_without_custom_properties("var(--missing, 1)px") else {
            panic!("expected fallback substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16(" 1/**/px"));
    }

    #[test]
    fn recognizes_substituted_css_wide_keywords() {
        assert!(is_single_css_wide_keyword(&tokenize_owned(b"  inherit ")));
        assert!(!is_single_css_wide_keyword(&tokenize_owned(b"inherit green")));
    }

    #[test]
    fn substitutes_raw_attributes_and_marks_taint() {
        let (TokenResolution::Resolved(tokens), contains_attr_tainted_values) =
            substitute_with_attributes("attr(data-value)", &[("data-value", "hello")])
        else {
            panic!("expected attribute substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("\"hello\""));
        assert!(contains_attr_tainted_values);
    }

    #[test]
    fn missing_untyped_attribute_becomes_an_empty_string() {
        let (TokenResolution::Resolved(tokens), contains_attr_tainted_values) =
            substitute_with_attributes("attr(data-value)", &[])
        else {
            panic!("expected missing-attribute substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("\"\""));
        assert!(!contains_attr_tainted_values);
    }

    #[test]
    fn accepts_all_dimension_units_and_rejects_unknown_units() {
        let Ok((_, AttrSyntax::Unit(unit))) = parse_attr_syntax(&tokenize_owned(b"data-value FR")) else {
            panic!("expected flex unit syntax");
        };
        assert_eq!(unit, utf16("FR"));
        assert!(parse_attr_syntax(&tokenize_owned(b"data-value unknown-unit")).is_err());
    }

    #[test]
    fn html_attribute_names_are_ascii_case_insensitive() {
        let attributes = HashMap::from([(utf16("data-value"), utf16("hello"))]);
        let mut context = ASFResolutionContext {
            attributes: Some(&attributes),
            attribute_names_are_ascii_case_insensitive: true,
            ..Default::default()
        };
        let TokenResolution::Resolved(tokens) =
            substitute_tokens(None, None, &tokenize_owned(b"attr(DATA-VALUE)"), &mut context, 0)
        else {
            panic!("expected case-insensitive attribute substitution");
        };
        assert_eq!(serialize_tokens(&tokens), utf16("\"hello\""));
    }

    #[test]
    fn deeply_nested_functions_are_invalid() {
        let nesting = MAX_SUBSTITUTION_RECURSION_DEPTH + 1;
        let source = format!(
            "{}var(--missing, 1px){}",
            "calc(".repeat(nesting as usize),
            ")".repeat(nesting as usize)
        );
        assert!(matches!(
            substitute_without_custom_properties(&source),
            TokenResolution::Invalid
        ));
    }
}

impl CustomPropertyRegistry {
    /// The registry of a document that registers no custom property.
    pub(crate) fn empty() -> Self {
        Self {
            registrations: HashMap::new(),
            document_url: Vec::new(),
            document_base_url: Vec::new(),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_custom_property_registry_create() -> *mut c_void {
    Box::into_raw(Box::new(CustomPropertyRegistry::empty())).cast()
}

/// Replaces the effective registered custom-property names for one document.
///
/// # Safety
/// `registry` must be a live pointer returned by `rust_custom_property_registry_create`, and
/// `registrations` must point at `registration_count` valid entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_registry_update(
    registry: *mut c_void,
    context: *const FfiCustomPropertyRegistryContext,
    registrations: *const FfiCustomPropertyRegistration,
    registration_count: usize,
) {
    let Some(context) = (unsafe { context.as_ref() }) else {
        return;
    };
    let registrations = if registration_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(registrations, registration_count) }
    };
    let registry = unsafe { &mut *registry.cast::<CustomPropertyRegistry>() };
    registry.document_url = unsafe { crate::bytes_from_raw(context.document_url, context.document_url_length) }
        .unwrap_or_default()
        .to_vec();
    registry.document_base_url =
        unsafe { crate::bytes_from_raw(context.document_base_url, context.document_base_url_length) }
            .unwrap_or_default()
            .to_vec();
    registry.registrations.clear();
    registry.registrations.reserve(registrations.len());
    for registration in registrations {
        let name = unsafe { registration.name.to_utf16() }.expect("invalid registered custom property name");
        let initial_source = if registration.has_initial_value {
            Some(
                unsafe { registration.initial_value.to_utf16() }
                    .expect("invalid registered custom property initial value"),
            )
        } else {
            None
        };
        let Some(syntax) = (unsafe { clone_syntax_handle(registration.syntax) }) else {
            continue;
        };
        // SAFETY: a published computed initial value is a live style value for the call; the
        //         registry holds one reference of its own for as long as it names the registration.
        let computed_initial = (!registration.computed_initial_value.is_null()).then(|| unsafe {
            RetainedStyleValueData::from_retained_pointer(crate::css::style_value::retain_style_value(
                registration.computed_initial_value.cast(),
            ))
        });
        registry.registrations.insert(
            name,
            RegisteredCustomProperty {
                syntax,
                inherits: registration.inherits,
                initial_source,
                computed_initial,
            },
        );
    }
}

/// # Safety
/// `registry` must be a pointer returned by `rust_custom_property_registry_create` that has not
/// already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_registry_destroy(registry: *mut c_void) {
    drop(unsafe { Box::from_raw(registry.cast::<CustomPropertyRegistry>()) });
}

/// Creates one Rust store node. Each entry transfers a leaked fly-string reference and a
/// strong style value data handle. The structural and inheritance parents are other Arc raw
/// pointers; they can differ when the C++ store has compacted its structural chain.
///
/// # Safety
/// `entries` must point at `entry_count` valid entries. Both parent pointers must be null or
/// pointers returned by this function that remain live for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_create(
    entries: *const FfiCustomPropertyStoreEntry,
    entry_count: usize,
    declared_count: usize,
    parent: *const c_void,
    inheritance_parent: *const c_void,
) -> *const c_void {
    crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::CustomPropertyStoreLifecycleEntry);
    let entries = if entry_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(entries, entry_count) }
    };
    assert!(declared_count <= entries.len());
    let parent = if parent.is_null() {
        None
    } else {
        let parent = parent.cast::<CustomPropertyStore>();
        unsafe { Arc::increment_strong_count(parent) };
        Some(unsafe { Arc::from_raw(parent) })
    };
    let inheritance_parent = if inheritance_parent.is_null() {
        None
    } else {
        let inheritance_parent = inheritance_parent.cast::<CustomPropertyStore>();
        unsafe { Arc::increment_strong_count(inheritance_parent) };
        Some(unsafe { Arc::from_raw(inheritance_parent) })
    };
    let mut own_names = HashMap::with_capacity(entries.len());
    let own_values = entries
        .iter()
        .map(|entry| {
            let name: Arc<[u16]> = unsafe { entry.name.to_utf16() }
                .expect("invalid custom property name")
                .into();
            own_names.insert(name.clone(), entry.name_raw);
            (
                entry.name_raw,
                CustomPropertyEntry {
                    _name: unsafe { RetainedUtf16FlyString::from_leaked_raw(entry.name_raw) },
                    name,
                    value: unsafe { RetainedStyleValueData::from_retained_pointer(entry.data.cast()) },
                    important: entry.important,
                },
            )
        })
        .collect();
    Arc::into_raw(Arc::new(CustomPropertyStore {
        own_values,
        declared_names: entries[..declared_count].iter().map(|entry| entry.name_raw).collect(),
        own_names,
        ancestor_count: parent.as_ref().map_or(0, |parent| parent.ancestor_count + 1),
        parent,
        inheritance_parent,
    }))
    .cast()
}

/// Creates a store holding an element's animated custom property values on top of the element's
/// own environment store. Transfers one strong reference per entry value and returns one strong
/// store reference.
///
/// # Safety
/// `entries` must point to `entry_count` valid entries whose `data` pointers are retained
/// style value references. `base` must be null or a live store reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_create_animation_overlay(
    entries: *const FfiCustomPropertyStoreEntry,
    entry_count: usize,
    base: *const c_void,
) -> *const c_void {
    crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::CustomPropertyStoreLifecycleEntry);
    let entries = if entry_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(entries, entry_count) }
    };
    let base = if base.is_null() {
        None
    } else {
        let base = base.cast::<CustomPropertyStore>();
        Some(unsafe { &*base })
    };
    let (mut own_values, mut own_names, parent, inheritance_parent, ancestor_count) = match base {
        Some(base) => (
            base.own_values.clone(),
            base.own_names.clone(),
            base.parent.clone(),
            base.inheritance_parent.clone(),
            base.ancestor_count,
        ),
        None => (HashMap::new(), HashMap::new(), None, None, 0),
    };
    let mut declared_names = Vec::with_capacity(entries.len());
    for entry in entries {
        let name: Arc<[u16]> = unsafe { entry.name.to_utf16() }
            .expect("invalid custom property name")
            .into();
        declared_names.push(entry.name_raw);
        own_names.insert(name.clone(), entry.name_raw);
        own_values.insert(
            entry.name_raw,
            CustomPropertyEntry {
                _name: unsafe { RetainedUtf16FlyString::from_leaked_raw(entry.name_raw) },
                name,
                value: unsafe { RetainedStyleValueData::from_retained_pointer(entry.data.cast()) },
                important: entry.important,
            },
        );
    }
    Arc::into_raw(Arc::new(CustomPropertyStore {
        own_values,
        declared_names,
        own_names,
        ancestor_count,
        parent,
        inheritance_parent,
    }))
    .cast()
}

/// Releases one store reference returned by `rust_custom_property_store_create`.
///
/// # Safety
/// `store` must be a non-null pointer returned by `rust_custom_property_store_create` that has
/// not already been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_destroy(store: *const c_void) {
    crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::CustomPropertyStoreLifecycleEntry);
    drop(unsafe { Arc::from_raw(store.cast::<CustomPropertyStore>()) });
}

/// Retains one strong reference to a custom-property store.
///
/// # Safety
/// `store` must be a live store pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_retain(store: *const c_void) -> *const c_void {
    unsafe { Arc::increment_strong_count(store.cast::<CustomPropertyStore>()) };
    store
}

/// Filter the host's layer using its published registration decisions, sharing the engine's
/// inheritance operation. The result transfers one store reference, or is null when empty.
///
/// # Safety
/// `store` is live; `parent` is null or live; `excluded` holds `excluded_count` name atoms.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_inheritable_layer(
    store: *const c_void,
    parent: *const c_void,
    excluded: *const usize,
    excluded_count: usize,
) -> *const c_void {
    let excluded = if excluded_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(excluded, excluded_count) }
    };
    unsafe { CustomPropertyStore::inheritable_layer(store.cast(), parent, excluded) }
}

/// Flatten a store for a host wrapper without promoting inherited names into its declared
/// prefix or losing the original inheritance parent used by explicit inheritance.
///
/// # Safety
/// `store` must be live. The result transfers one strong store reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_flatten(store: *const c_void) -> *const c_void {
    let source = unsafe { &*store.cast::<CustomPropertyStore>() };
    let mut own_values = source.own_values.clone();
    let mut own_names = source.own_names.clone();
    let mut parent = source.parent.as_deref();
    while let Some(current) = parent {
        for (&name, entry) in &current.own_values {
            if let std::collections::hash_map::Entry::Vacant(slot) = own_values.entry(name) {
                own_names.insert(entry.name.clone(), name);
                slot.insert(entry.clone());
            }
        }
        parent = current.parent.as_deref();
    }
    Arc::into_raw(Arc::new(CustomPropertyStore {
        own_values,
        own_names,
        declared_names: source.declared_names.clone(),
        inheritance_parent: source.inheritance_parent.clone(),
        parent: None,
        ancestor_count: 0,
    }))
    .cast()
}

/// Hands every effective custom property to `callback`, with nearer entries shadowing ancestors.
///
/// # Safety
/// `store` must be a live store pointer, and `callback` must not retain a value past the call
/// without retaining it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_for_each_effective_entry(
    store: *const c_void,
    context: *mut c_void,
    callback: unsafe extern "C" fn(*mut c_void, usize, bool, *const c_void),
) {
    let mut seen = std::collections::HashSet::new();
    let mut current = Some(unsafe { &*store.cast::<CustomPropertyStore>() });
    while let Some(store) = current {
        for (&name_raw, entry) in &store.own_values {
            if seen.insert(name_raw) {
                unsafe { callback(context, name_raw, entry.important, entry.value.pointer().cast()) };
            }
        }
        current = store.parent.as_deref();
    }
}

/// Hands every custom property a store declares itself to `callback`, in declaration order, with
/// the fly string it is named by and a borrowed value.
///
/// # Safety
/// `store` must be a live store pointer, and `callback` must not retain the value past the call
/// without retaining it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_custom_property_store_for_each_own_entry(
    store: *const c_void,
    context: *mut c_void,
    callback: unsafe extern "C" fn(*mut c_void, usize, bool, *const c_void),
) {
    let store = unsafe { &*store.cast::<CustomPropertyStore>() };
    for name_raw in &store.declared_names {
        let entry = store
            .own_values
            .get(name_raw)
            .expect("declared custom property must be an own value");
        unsafe {
            callback(context, *name_raw, entry.important, entry.value.pointer().cast());
        }
    }
}
