/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the engine needs to sample an element's animations itself, from its own records and the
//! document's published inputs rather than from the host's working set.

use super::bridge::element_adjustment_fact;
use super::computed;
use super::publication::drive_font_metric;
use super::tree::StyleNodeID;
use super::{RetainedState, bridge};
use crate::css::animated_overlay::AnimatedOverlay;
use crate::css::computed_longhand_table::ComputedLonghandTable;
use crate::css::computed_value_types::{FontValues, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
use crate::css::computed_values::InheritedBoxValues;
use crate::css::style_compute::{FfiAnimationLengthContexts, FfiFontMetrics, FfiLengthResolutionContext};
use crate::css::style_value::StyleValueData;
use crate::css::table_group_builder::FfiFontGroupBuildInputs;

/// One record's font, as a length resolves against it.
struct RecordFont {
    metrics: FfiFontMetrics,
    depends_on_viewport_metrics: bool,
    inline_axis_is_horizontal: bool,
}

impl RetainedState {
    /// The record the element or one of its pseudo-elements holds, as the host installed it.
    #[must_use]
    pub(crate) fn assigned_style_record_of(&self, node: StyleNodeID, pseudo_kind: Option<u8>) -> Option<u64> {
        let pseudo_kind = pseudo_kind.unwrap_or(crate::css::cascaded_properties::NO_PSEUDO_ELEMENT);
        self.computed_group_sets
            .assigned_final_style_record(computed::ComputedStyleTarget::new(node, pseudo_kind))
            .map(computed::FinalStyleRecordID::raw)
    }

    fn record_font(&self, style_record: u64) -> Option<RecordFont> {
        let view = self.computed_group_sets.style_record_view(style_record)?;
        let font = unsafe { view.payloads[STYLE_GROUP_INDEX_FONT].cast::<FontValues>().deref() };
        let inherited_box = unsafe {
            view.payloads[STYLE_GROUP_INDEX_INHERITED_BOX]
                .cast::<InheritedBoxValues>()
                .deref()
        };
        Some(RecordFont {
            metrics: FfiFontMetrics {
                font_size: font.font_size.to_double(),
                x_height: drive_font_metric(font.font_x_height),
                cap_height: drive_font_metric(font.font_ascent),
                zero_advance: drive_font_metric(font.font_zero_advance),
                line_height: font.line_height_used.to_double(),
            },
            depends_on_viewport_metrics: view.dependency_flags & (1 << 1) != 0,
            inline_axis_is_horizontal: inherited_box.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB,
        })
    }

    /// The definition the last plan applied to each CSS animation of one of an element's lists.
    pub(crate) fn element_applied_animation_definitions(
        &self,
        node: StyleNodeID,
        slot: super::animations::AnimationSlot,
    ) -> &[super::animations::AppliedAnimationDefinition] {
        self.css_defined_animations.definitions(node, slot)
    }

    /// The plan the row the pass settled for an element leaves for the host, while it is left.
    pub(crate) fn element_settled_animation_plan(
        &self,
        node: StyleNodeID,
    ) -> Option<&super::animations::SettledAnimationPlan> {
        self.nodes_owing_animation_definitions.get(&(node, u8::MAX))
    }

    /// Lend out the published `@keyframes`, which a sample reads the rules of brand-new animations
    /// from while it holds the engine, until `restore_animation_keyframes`.
    pub(crate) fn take_animation_keyframes(&mut self) -> super::animations::AnimationKeyframes {
        std::mem::take(&mut self.animation_keyframes)
    }

    pub(crate) fn restore_animation_keyframes(&mut self, keyframes: super::animations::AnimationKeyframes) {
        self.animation_keyframes = keyframes;
    }

    pub(crate) fn root_element_font_metrics(&self) -> super::animations::RootElementFontMetrics {
        self.root_element_font_metrics
    }

    pub(crate) fn document_style_computation_inputs(&self) -> bridge::FfiDocumentStyleComputationInputs {
        self.document_style_computation_inputs
    }

    /// Keep what the container units a sample over `style_record` resolved read of the element's
    /// containers for the host, as a record the engine computes does. Whether there was a record to
    /// read them over.
    pub(crate) fn note_sampled_container_unit_effects(
        &mut self,
        node: StyleNodeID,
        style_record: u64,
        mask: u8,
    ) -> bool {
        let Some(record) = computed::FinalStyleRecordID::from_raw(style_record) else {
            return false;
        };
        self.note_container_unit_effects_for_host(node, record, mask);
        true
    }

    /// The record the engine assigned the element's inheritance parent: its originating element,
    /// for a pseudo-element.
    pub(crate) fn assigned_inheritance_parent_record(&self, node: StyleNodeID, pseudo_kind: Option<u8>) -> Option<u64> {
        let parent = match pseudo_kind {
            Some(_) => Some(node),
            None => self.tree.inheritance_parent(node),
        }?;
        self.computed_group_sets
            .assigned_style_record(parent)
            .map(computed::FinalStyleRecordID::raw)
    }

    /// The font metrics a `rem` resolves against where the document element holds the record the
    /// engine assigned it, which a pass can settle before the host installs it.
    pub(crate) fn assigned_root_element_font_metrics(
        &self,
        root: StyleNodeID,
    ) -> Option<super::animations::RootElementFontMetrics> {
        let record = self.computed_group_sets.assigned_style_record(root)?;
        let inputs = self.root_font_inputs_from_raw_record(record.raw())?;
        Some(super::animations::RootElementFontMetrics::from_words(
            &inputs.metrics,
            inputs.depends_on_viewport,
        ))
    }

    /// The three length-resolution contexts a sample of the element's animations computes keyframe
    /// values in, over `style_record` and its inheritance parent's `parent_record`: the font context,
    /// which reads the inheritance parent; the line-height context, which reads the element's own
    /// font and the parent's line height; and the one everything else resolves against, the
    /// element's own font. A mirror of the host's `get_computation_context_for_property(FontFamily /
    /// LineHeight / Color)` over the working set it reconstructs from that record, with the
    /// container bases `container_unit_mask` asks for and `rem` resolving against `root`.
    ///
    /// `None` where the element holds no record the engine can read.
    pub(crate) fn animation_sample_length_contexts(
        &self,
        node: StyleNodeID,
        pseudo_kind: Option<u8>,
        style_record: u64,
        parent_record: Option<u64>,
        container_unit_mask: u8,
        root: super::animations::RootElementFontMetrics,
    ) -> Option<FfiAnimationLengthContexts> {
        let inputs: &bridge::FfiDocumentStyleComputationInputs = &self.document_style_computation_inputs;
        let own = self.record_font(style_record)?;
        let parent_font = parent_record.and_then(|record| self.record_font(record));
        let is_document_element =
            self.computed_group_sets.adjustment_facts(node) & element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0;
        let subject_inline_axis_is_horizontal = own.inline_axis_is_horizontal;
        let (width_basis, height_basis) =
            self.container_unit_bases(node, container_unit_mask, subject_inline_axis_is_horizontal, inputs);
        // What a `rem` resolves against is the font of the record the host last installed on the
        // document element, which is what the host's own member holds - not the document inputs,
        // which were published before this update installed anything - unless the caller names
        // the record a pass settled for it.
        let document_root_font_metrics = FfiFontMetrics {
            font_size: root.font_size,
            x_height: root.x_height,
            cap_height: root.cap_height,
            zero_advance: root.zero_advance,
            line_height: root.line_height,
        };
        let length_context =
            |font_metrics: FfiFontMetrics,
             font_metrics_depend_on_viewport_metrics: bool,
             root_font_metrics: FfiFontMetrics,
             root_font_metrics_depend_on_viewport_metrics: bool| FfiLengthResolutionContext {
                viewport_width: inputs.viewport_width,
                viewport_height: inputs.viewport_height,
                font_metrics,
                root_font_metrics,
                font_metrics_depend_on_viewport_metrics,
                root_font_metrics_depend_on_viewport_metrics,
                has_container_width_basis: width_basis.is_some(),
                has_container_height_basis: height_basis.is_some(),
                container_width_basis: width_basis.map_or(0.0, |basis| basis.basis),
                container_height_basis: height_basis.map_or(0.0, |basis| basis.basis),
                container_width_basis_depends_on_viewport_metrics: width_basis
                    .is_some_and(|basis| basis.depends_on_viewport_metrics),
                container_height_basis_depends_on_viewport_metrics: height_basis
                    .is_some_and(|basis| basis.depends_on_viewport_metrics),
                subject_inline_axis_is_horizontal,
                resolved_viewport_relative_length: std::ptr::null_mut(),
            };

        // The font context is the parent's own, or the document's initial font's where there is no
        // parent to read. The initial line height is zero.
        let initial_metrics = FfiFontMetrics {
            font_size: inputs.initial_font_size,
            x_height: inputs.initial_font_x_height,
            cap_height: inputs.initial_font_cap_height,
            zero_advance: inputs.initial_font_zero_advance,
            line_height: 0.0,
        };
        let font = match &parent_font {
            Some(parent_font) => length_context(
                parent_font.metrics,
                parent_font.depends_on_viewport_metrics,
                document_root_font_metrics,
                root.depends_on_viewport_metrics,
            ),
            None => length_context(initial_metrics, false, initial_metrics, false),
        };

        // The line-height context is the element's own font with the line height it inherits.
        let line_height_metrics = FfiFontMetrics {
            line_height: parent_font
                .as_ref()
                .map_or(0.0, |parent_font| parent_font.metrics.line_height),
            ..own.metrics
        };
        let line_height = match is_document_element {
            true => length_context(
                line_height_metrics,
                own.depends_on_viewport_metrics,
                line_height_metrics,
                own.depends_on_viewport_metrics,
            ),
            false => length_context(
                line_height_metrics,
                own.depends_on_viewport_metrics,
                document_root_font_metrics,
                root.depends_on_viewport_metrics,
            ),
        };

        // `rem` on the document element itself names its own font.
        let remaining = match is_document_element && pseudo_kind.is_none() {
            true => length_context(
                own.metrics,
                own.depends_on_viewport_metrics,
                own.metrics,
                own.depends_on_viewport_metrics,
            ),
            false => length_context(
                own.metrics,
                own.depends_on_viewport_metrics,
                document_root_font_metrics,
                root.depends_on_viewport_metrics,
            ),
        };
        Some(FfiAnimationLengthContexts {
            font,
            line_height,
            remaining,
        })
    }
}

/// What resolving an element's font asks the document's font resolver, and the three values the
/// font's metrics are then read beside, from a computed table and the overlay a sample composed over
/// it. The drive asks with no overlay.
pub(crate) struct FontResolutionInputs {
    pub(crate) request: bridge::FfiFontResolutionRequest,
    /// The font size the element's own lengths resolve against, which is the C++ working set's
    /// `CSSPixels` value rather than the computed value's double.
    pub(crate) font_size: f64,
    pub(crate) font_weight: f64,
    pub(crate) font_width: f64,
}

fn effective_data<'a>(
    table: &'a ComputedLonghandTable,
    overlay: Option<&'a AnimatedOverlay>,
    property: u16,
) -> Option<&'a StyleValueData> {
    unsafe {
        table
            .effective_value(overlay, property, true)
            .value
            .cast::<StyleValueData>()
            .as_ref()
    }
}

/// The request the C++ font computer's resolution corresponds to for these values.
pub(crate) fn font_resolution_inputs(
    table: &ComputedLonghandTable,
    overlay: Option<&AnimatedOverlay>,
    tree_scope: u32,
    inputs: &bridge::FfiDocumentStyleComputationInputs,
) -> FontResolutionInputs {
    use crate::css::css_pixels::CssPixels;
    use crate::css::property_metadata::property_id as prop;
    use crate::css::style_compute::keyword;

    // The font phase computes font-size, font-weight, and font-width to these types: a value it
    // cannot compute is `unset`.
    let font_size = match effective_data(table, overlay, prop::FONT_SIZE) {
        Some(StyleValueData::Length { value, unit }) if *unit == crate::css::style_compute::px_length_unit() => {
            CssPixels::nearest_value_for(*value).to_double()
        }
        _ => {
            debug_assert!(false, "the font phase left font-size uncomputed");
            CssPixels::from_raw(inputs.initial_font_size_raw).to_double()
        }
    };
    let font_slope = match effective_data(table, overlay, prop::FONT_STYLE) {
        Some(StyleValueData::FontStyle { font_style, .. }) => match *font_style {
            crate::css::css_enums::font_style_keyword::ITALIC => 1,
            crate::css::css_enums::font_style_keyword::OBLIQUE => 2,
            _ => 0,
        },
        _ => 0,
    };
    let (font_weight, font_width) = match (
        effective_data(table, overlay, prop::FONT_WEIGHT),
        effective_data(table, overlay, prop::FONT_WIDTH),
    ) {
        (Some(StyleValueData::Number { value: weight }), Some(StyleValueData::Percentage { value: width })) => {
            (*weight, *width)
        }
        _ => {
            debug_assert!(false, "the font phase left font-weight or font-width uncomputed");
            (400.0, 100.0)
        }
    };
    let font_optical_sizing = match effective_data(table, overlay, prop::FONT_OPTICAL_SIZING) {
        Some(StyleValueData::Keyword { keyword }) => {
            crate::css::css_enums::keyword_to_font_optical_sizing(*keyword).unwrap_or(0)
        }
        _ => 0,
    };
    // The resolver reads these beside the family, so the request names each one whose value is not
    // the initial one and nothing for the rest. Read the computed values: a non-default setting can
    // also come from inheritance.
    let font_feature_values: [bridge::FfiHostHandle; bridge::FONT_RESOLUTION_FEATURE_INPUT_COUNT] = {
        let features = [
            (prop::FONT_FEATURE_SETTINGS, keyword::NORMAL),
            (prop::FONT_VARIATION_SETTINGS, keyword::NORMAL),
            (prop::FONT_VARIANT_CAPS, keyword::NORMAL),
            (prop::FONT_VARIANT_EAST_ASIAN, keyword::NORMAL),
            (prop::FONT_VARIANT_EMOJI, keyword::NORMAL),
            (prop::FONT_VARIANT_LIGATURES, keyword::NORMAL),
            (prop::FONT_VARIANT_NUMERIC, keyword::NORMAL),
            (prop::FONT_VARIANT_POSITION, keyword::NORMAL),
            (prop::FONT_VARIANT_ALTERNATES, keyword::NORMAL),
            (prop::FONT_KERNING, keyword::AUTO),
            (prop::TEXT_RENDERING, keyword::AUTO),
        ];
        std::array::from_fn(|index| {
            let (property, default_keyword) = features[index];
            let pointer = match effective_data(table, overlay, property) {
                Some(StyleValueData::Keyword { keyword }) if *keyword == default_keyword => std::ptr::null(),
                _ => table.effective_value(overlay, property, true).value,
            };
            bridge::FfiHostHandle::from_pointer(pointer.cast())
        })
    };
    FontResolutionInputs {
        request: bridge::FfiFontResolutionRequest {
            font_family: bridge::FfiHostHandle::from_pointer(
                table.effective_value(overlay, prop::FONT_FAMILY, true).value.cast(),
            ),
            tree_scope,
            font_feature_values,
            font_size_raw: CssPixels::nearest_value_for(font_size).raw_value(),
            font_slope,
            font_weight,
            font_width,
            font_optical_sizing,
            font_environment_generation: inputs.font_environment_generation,
        },
        font_size,
        font_weight,
        font_width,
    }
}

/// The used line height, as the C++ working set reads it from the computed value against the font
/// it resolved.
pub(crate) fn used_line_height(
    table: &ComputedLonghandTable,
    overlay: Option<&AnimatedOverlay>,
    font_size: f64,
    resolved: &bridge::FfiResolvedFont,
) -> f64 {
    use crate::css::css_pixels::CssPixels;
    use crate::css::property_metadata::property_id as prop;
    use crate::css::style_compute::keyword;

    let normal_line_height = f64::from(resolved.ascent.round() as i32 + resolved.descent.round() as i32);
    // The line-height phase computes line-height to one of these; a value it cannot compute is
    // `unset`, so any other shape is read as `normal`.
    match effective_data(table, overlay, prop::LINE_HEIGHT) {
        Some(StyleValueData::Keyword { keyword }) if *keyword == keyword::NORMAL => normal_line_height,
        Some(StyleValueData::Length { value, unit }) if *unit == crate::css::style_compute::px_length_unit() => {
            CssPixels::nearest_value_for(*value).to_double()
        }
        Some(StyleValueData::Number { value }) => CssPixels::nearest_value_for(value * font_size).to_double(),
        _ => {
            debug_assert!(false, "the line-height phase left line-height uncomputed");
            normal_line_height
        }
    }
}

/// What the font group of a record is built from, for an element whose font resolved to `resolved`.
pub(crate) fn font_group_build_inputs(
    table: &ComputedLonghandTable,
    overlay: Option<&AnimatedOverlay>,
    font: &FontResolutionInputs,
    line_height_used: f64,
    resolved: &bridge::FfiResolvedFont,
) -> FfiFontGroupBuildInputs {
    use crate::css::css_pixels::CssPixels;
    use crate::css::property_metadata::property_id as prop;

    let keyword_code = |property: u16, map: fn(u16) -> Option<u8>| match effective_data(table, overlay, property) {
        Some(StyleValueData::Keyword { keyword }) => map(*keyword).unwrap_or(0),
        _ => 0,
    };
    let math_depth = match effective_data(table, overlay, prop::MATH_DEPTH) {
        Some(StyleValueData::Integer { value }) => *value,
        _ => 0,
    };
    FfiFontGroupBuildInputs {
        font_size_raw: font.request.font_size_raw,
        line_height_used_raw: CssPixels::nearest_value_for(line_height_used).raw_value(),
        font_variant_emoji: keyword_code(
            prop::FONT_VARIANT_EMOJI,
            crate::css::css_enums::keyword_to_font_variant_emoji,
        ),
        font_ascent: resolved.ascent,
        font_descent: resolved.descent,
        font_x_height: resolved.x_height,
        font_zero_advance: resolved.zero_advance,
        first_available_font: resolved.first_available_font.as_pointer(),
        font_cascade_list: resolved.font_cascade_list.as_pointer(),
        font_weight: font.font_weight,
        font_width: font.font_width,
        math_shift: keyword_code(prop::MATH_SHIFT, crate::css::css_enums::keyword_to_math_shift),
        math_style: keyword_code(prop::MATH_STYLE, crate::css::css_enums::keyword_to_math_style),
        math_depth,
    }
}

impl RetainedState {
    /// The font the document's font resolver resolved for a request, if it has.
    pub(crate) fn resolved_font(&self, request: bridge::FfiFontResolutionRequest) -> Option<bridge::FfiResolvedFont> {
        self.font_resolution.as_ref()?.lookup(request)
    }

    /// The inputs the font group of an element's overlay record is built from, resolved by the
    /// engine over the record's table and the overlay a sample composed. `None` where the document's
    /// font resolver has not resolved that font yet.
    pub(crate) fn animated_font_group_inputs(
        &self,
        node: StyleNodeID,
        table: &ComputedLonghandTable,
        overlay: Option<&AnimatedOverlay>,
    ) -> Option<FfiFontGroupBuildInputs> {
        let font = font_resolution_inputs(
            table,
            overlay,
            self.tree.tree_scope(node).0,
            &self.document_style_computation_inputs,
        );
        let resolved = self.font_resolution.as_ref()?.lookup(font.request)?;
        let line_height_used = used_line_height(table, overlay, font.font_size, &resolved);
        Some(font_group_build_inputs(
            table,
            overlay,
            &font,
            line_height_used,
            &resolved,
        ))
    }
}

/// The custom-property environments a sample of an element's animations reads: the element's own,
/// which may be the one a previous sample composed; the one its own declarations resolved to
/// beneath that composition; and the one it inherits. Each is a raw `Arc` pointer to a
/// `CustomPropertyStore`, or null.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SampleCustomPropertyEnvironments {
    pub(crate) store: *const std::ffi::c_void,
    pub(crate) base_store: *const std::ffi::c_void,
    pub(crate) inheritance_store: *const std::ffi::c_void,
    /// Whether the base is not simply what the element inherits.
    pub(crate) element_declares_own: bool,
    /// Whether the base is an environment the engine resolved.
    pub(crate) base_is_engine: bool,
}

/// `StyleEngine::is_engine_custom_property_environment`.
const ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG: u64 = 1 << 62;

impl RetainedState {
    /// The custom-property environments a sample of an element's animations reads, from the
    /// environments the engine holds for the element and its inheritance parent, or why the engine
    /// cannot say.
    pub(crate) fn animation_sample_custom_property_environments(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: Option<u8>,
    ) -> Result<SampleCustomPropertyEnvironments, &'static str> {
        const UNKNOWN_ELEMENT: &str = "an element the engine holds no environment for";
        if let Some(pseudo_kind) = pseudo_kind {
            return self.pseudo_element_sample_custom_property_environments(node, pseudo_kind);
        }
        let (environment, store) = self.element_custom_property_environment(node).ok_or(UNKNOWN_ELEMENT)?;
        let data = self.element_custom_property_data(node).0;
        // A sample composes the element's animated custom properties over the environment its own
        // declarations resolved to, and the element then holds the composition.
        let (base_environment, base_store, base_data) =
            self.element_custom_property_animation_base(node)
                .unwrap_or((environment, store, data));
        let parent = self.tree.inheritance_parent(node);
        let parent_environment = match parent {
            Some(parent) => Some(
                self.element_custom_property_environment(parent)
                    .ok_or(UNKNOWN_ELEMENT)?,
            ),
            None => None,
        };
        let inheritance_store = parent_environment.map_or(std::ptr::null(), |(_, store)| store);
        // Where custom properties are registered, the environment a parent passes on is a
        // projection of its own without the names that do not inherit, so whether the element's
        // base is what it inherits is what its own cascade says: whether it declares any.
        if self.custom_property_registry_has_registrations()? {
            return Ok(SampleCustomPropertyEnvironments {
                store,
                base_store,
                inheritance_store,
                element_declares_own: !base_store.is_null() && self.node_declares_custom_properties(node),
                base_is_engine: !base_store.is_null() && base_environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0,
            });
        }
        // The base is what the element inherits where it is the environment the parent passes on.
        // An environment the engine moved the parent to has no host object, so the one the element
        // holds, the host's view of it, is the parent's by identity.
        let base_is_inherited = match parent {
            Some(parent) => {
                let (parent_data, parent_environment) = self.element_custom_property_data(parent);
                parent_data == base_data
                    || (parent_data.is_null()
                        && base_environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0
                        && parent_environment == base_environment)
            }
            None => false,
        };
        let element_declares_own = !base_store.is_null() && !base_is_inherited;
        Ok(SampleCustomPropertyEnvironments {
            store,
            base_store,
            inheritance_store,
            element_declares_own,
            base_is_engine: !base_store.is_null() && base_environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0,
        })
    }

    /// The custom-property environments a sample of one of an element's synthetic pseudo-elements
    /// reads: the pseudo-element's own, the one its own style resolved to beneath what its
    /// animations composed, and its originating element's, which it inherits.
    fn pseudo_element_sample_custom_property_environments(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Result<SampleCustomPropertyEnvironments, &'static str> {
        let (store, base_environment, base_store, declares_own) = self
            .pseudo_element_custom_property_sample_inputs(node, pseudo_kind)
            .ok_or("a pseudo-element environment without a store")?;
        let (_, inheritance_store) = self
            .element_custom_property_environment(node)
            .ok_or("an element the engine holds no environment for")?;
        Ok(SampleCustomPropertyEnvironments {
            store,
            base_store,
            inheritance_store,
            element_declares_own: !base_store.is_null() && declares_own,
            base_is_engine: !base_store.is_null() && base_environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0,
        })
    }

    /// Whether the document registers custom properties, or why the engine cannot say.
    fn custom_property_registry_has_registrations(&self) -> Result<bool, &'static str> {
        // SAFETY: The document keeps the registry it published alive while the engine holds it.
        let registry = unsafe {
            self.document_style_computation_inputs
                .custom_property_registry
                .as_pointer()
                .cast::<crate::css::custom_properties::CustomPropertyRegistry>()
                .as_ref()
        };
        registry
            .map(crate::css::custom_properties::CustomPropertyRegistry::has_registrations)
            .ok_or("a document that published no registry")
    }

    /// The custom-property environments a sample of a row the pass settled reads, from the
    /// environment its new record was published with and the one its inheritance parent passes on,
    /// before the host installs either; or why the engine cannot say.
    pub(crate) fn settled_row_custom_property_environments(
        &self,
        node: StyleNodeID,
    ) -> Result<SampleCustomPropertyEnvironments, &'static str> {
        // The environment an earlier sample composed the element's animated custom properties into
        // is the host's until the engine owns animation overlays.
        if self.sampled_custom_property_environments.contains_key(&node) {
            return Err("custom properties an earlier sample animated");
        }
        let has_registrations = self.custom_property_registry_has_registrations()?;
        let store_of = |environment: u64| match environment {
            0 => Ok(std::ptr::null()),
            environment => self
                .custom_property_environments
                .store(environment)
                .ok_or("an environment without a store"),
        };
        let environment = self
            .computed_group_sets
            .custom_property_environment_identity(node)
            .unwrap_or(0);
        let store = store_of(environment)?;
        let parent_environment = self
            .tree
            .inheritance_parent(node)
            .and_then(|parent| self.computed_group_sets.custom_property_environment_identity(parent))
            .unwrap_or(0);
        Ok(SampleCustomPropertyEnvironments {
            store,
            base_store: store,
            inheritance_store: store_of(parent_environment)?,
            // A parent passes on a projection of its environment where custom properties are
            // registered, so the element's own cascade says whether it declares its own.
            element_declares_own: environment != 0
                && match has_registrations {
                    true => self.node_declares_custom_properties(node),
                    false => environment != parent_environment,
                },
            base_is_engine: environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0,
        })
    }
}

/// What the engine's own sample of an element leaves for the overlay record: the table after the
/// animated box-type finalization, and the overlay.
pub(crate) struct EngineSampledStyle {
    pub(crate) table: *mut crate::css::computed_longhand_table::ComputedLonghandTable,
    pub(crate) overlay: *mut crate::css::animated_overlay::AnimatedOverlay,
}

impl Drop for EngineSampledStyle {
    fn drop(&mut self) {
        unsafe {
            crate::css::computed_longhand_table::rust_computed_longhand_table_release(self.table);
            crate::css::animated_overlay::rust_animated_overlay_free(self.overlay);
        }
    }
}

/// What the pass published for a row whose animations it sampled itself, for the host to apply
/// where it installs the row: the composition, what publishing it invalidated, and what the sample
/// found out that the host records on the element and its parent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SettledRowPublication {
    pub(crate) style_record: u64,
    pub(crate) invalidation: bridge::FfiAnimationInvalidation,
    pub(crate) overlay_is_empty: bool,
    pub(crate) substitution_marks: u8,
    pub(crate) keyframes_inherited_non_inherited_style_groups: u32,
    pub(crate) uses_tree_counting_function: bool,
}

impl super::StyleEngineState {
    /// The record the host holds for an element, as it last installed one.
    pub(crate) fn held_style_record(&self, node: StyleNodeID) -> Option<u64> {
        self.host.held_style_records.get(&node).copied()
    }

    /// Publish what the pass's sample of a settled row composed as the element's overlay record,
    /// which the rows after it inherit from, and keep what the host applies when it installs the
    /// row. Or why the engine cannot, where the host samples the row itself.
    pub(crate) fn publish_settled_row_sample(
        &mut self,
        node: StyleNodeID,
        sample: crate::css::style_compute::SettledRowSample,
        counters: &mut super::Counters,
    ) -> Result<SettledRowPublication, &'static str> {
        use crate::css::computed_value_views::ComputedValuesView;
        use crate::css::host_shared::{HostShared, SharedPayload};
        use crate::css::property_metadata::property_id;

        let style_record = sample.style_record;
        let overlay = unsafe { &*sample.style.overlay };
        // The host resolves an animated color scheme and an animated display's pre-transformation
        // value from its working set.
        if overlay.get(property_id::COLOR_SCHEME).is_some() || overlay.get(property_id::DISPLAY).is_some() {
            return Err("an animated color scheme or display");
        }
        let (used_color_scheme, display_before_box_type_transformation) = {
            let view = self
                .computed_group_sets
                .style_record_view(style_record)
                .ok_or("a record with no view")?;
            let scheme =
                unsafe { view.longhand_table.as_ref() }.map_or(-1, ComputedLonghandTable::effective_color_scheme);
            let base_payloads = match view.base_payloads.is_empty() {
                true => view.payloads,
                false => view.base_payloads,
            };
            let display = ComputedValuesView::new(SharedPayload::as_pointer_slice(base_payloads))
                .display_before_box_type_transformation()
                .encoded();
            (
                u8::try_from(scheme).map_err(|_| "a record with no effective color scheme")?,
                display,
            )
        };
        let table = unsafe { &*sample.style.table };
        let font_unresolved = std::cell::Cell::new(false);
        let payloads = {
            let retained = &self.retained;
            let mut font = || {
                let inputs = retained.animated_font_group_inputs(node, table, Some(overlay));
                font_unresolved.set(inputs.is_none());
                inputs
            };
            unsafe {
                self.build_animation_overlay_payloads(
                    node,
                    u8::MAX,
                    style_record,
                    table,
                    Some(overlay),
                    used_color_scheme,
                    display_before_box_type_transformation,
                    &mut font,
                )
            }
            .ok_or(match font_unresolved.get() {
                true => "an overlay font nobody resolved yet",
                false => "no record to compose over",
            })?
        };
        let shared = SharedPayload::from_pointer_slice(&payloads.payloads);
        let is_document_element =
            self.computed_group_sets.adjustment_facts(node) & element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0;
        let invalidation =
            self.retained
                .compare_animation_overlay(style_record, sample.style.overlay, shared, is_document_element);
        let overlay_is_empty = overlay.is_empty();
        let identity = match overlay_is_empty {
            true => 0,
            false => {
                self.retained.next_engine_animation_overlay_identity += 1;
                (1 << 63) | self.retained.next_engine_animation_overlay_identity
            }
        };
        let publication = self
            .publish_animation_overlay_impl(
                computed::ComputedStyleTarget::new(node, u8::MAX),
                identity,
                HostShared::new(sample.style.overlay.cast_const()),
                if overlay_is_empty { &[] } else { shared },
                counters,
            )
            .ok_or("no overlay publication")?;
        let style_record = publication.style_record.raw();
        self.retained
            .computed_group_sets
            .set_sampled_composition_identity(node, style_record);
        // A sample the host publishes before it installs the row replaces the composition in its
        // slot, so the batch keeps it alive for the row until the host acknowledges it.
        if !overlay_is_empty {
            self.retained.computed_group_sets.pin_style_record(style_record);
            self.retained.batch_pinned_compositions.push((node, style_record));
        }
        let published = SettledRowPublication {
            style_record,
            invalidation,
            overlay_is_empty,
            substitution_marks: sample.substitution_marks,
            keyframes_inherited_non_inherited_style_groups: sample.keyframes_inherited_non_inherited_style_groups,
            uses_tree_counting_function: sample.uses_tree_counting_function,
        };
        self.retained.rows_sampled_in_pass.insert(node, published);
        Ok(published)
    }

    /// Take what the pass published for a row whose animations it sampled, so that exactly one
    /// installation applies it.
    pub(crate) fn take_row_sampled_in_pass(&mut self, node: StyleNodeID) -> Option<SettledRowPublication> {
        self.retained.rows_sampled_in_pass.remove(&node)
    }
}
