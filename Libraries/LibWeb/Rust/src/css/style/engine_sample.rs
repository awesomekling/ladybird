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
use super::{RetainedState, bridge, custom_property_environments, engine_sample_check, inputs};
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
        let has_registrations = self.custom_property_registry_has_registrations()?;
        let store_of = |environment: u64| match environment {
            0 => Ok(std::ptr::null()),
            environment => self
                .custom_property_environments
                .store(environment)
                .ok_or("an environment without a store"),
        };
        let environment = self.element_base_custom_property_environment(node)?;
        // What an earlier sample composed the element's animated custom properties into is the
        // environment the element holds while it is composed over the one its record resolved.
        let store = store_of(self.substitution_environment(node, environment))?;
        let base_store = store_of(environment)?;
        let parent_environment = self
            .tree
            .inheritance_parent(node)
            .and_then(|parent| self.computed_group_sets.custom_property_environment_identity(parent))
            .unwrap_or(0);
        Ok(SampleCustomPropertyEnvironments {
            store,
            base_store,
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

impl RetainedState {
    /// The custom-property environments a sample of a synthetic pseudo-element the engine settled
    /// reads, from the environment its new record was published with and its originating
    /// element's, which it inherits; or why the engine cannot say.
    pub(crate) fn settled_pseudo_element_custom_property_environments(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
        style_record: u64,
    ) -> Result<SampleCustomPropertyEnvironments, &'static str> {
        let store_of = |environment: u64| match environment {
            0 => Ok(std::ptr::null()),
            environment => self
                .custom_property_environments
                .store(environment)
                .ok_or("an environment without a store"),
        };
        // A composition is published over its base with the base's environment.
        let environment = self
            .computed_group_sets
            .style_record_custom_property_environment(self.computed_group_sets.base_style_record_of(style_record))
            .ok_or("a record with no view")?;
        let element_environment = self
            .computed_group_sets
            .custom_property_environment_identity(node)
            .unwrap_or(0);
        // Where a registration keeps custom properties from inheriting, what the element passes on
        // is a projection of its environment, which a pseudo-element declaring none holds.
        let inherited = self
            .built_inheritable_custom_property_environment(element_environment, &self.document_style_computation_inputs)
            .ok_or("an element environment whose inherited projection nobody built")?;
        let base_store = store_of(environment)?;
        // What an earlier sample composed the pseudo-element's animated custom properties into, over
        // the environment its record resolved, is the one its own values substitute under.
        let composed = self
            .sampled_pseudo_element_custom_property_environments
            .get(&(node, pseudo_kind))
            .copied()
            .filter(|&sampled| self.sampled_environment_is_over(sampled, environment))
            .and_then(|sampled| self.custom_property_environments.store(sampled))
            .or_else(|| {
                // One the host composed over the same environment.
                let (store, held_base, held_base_store, _) =
                    self.pseudo_element_custom_property_sample_inputs(node, pseudo_kind)?;
                (held_base == environment && store != held_base_store && !store.is_null()).then_some(store)
            });
        Ok(SampleCustomPropertyEnvironments {
            store: composed.unwrap_or(base_store),
            base_store,
            inheritance_store: store_of(element_environment)?,
            element_declares_own: environment != 0 && environment != element_environment && environment != inherited,
            base_is_engine: environment & ENGINE_CUSTOM_PROPERTY_ENVIRONMENT_TAG != 0,
        })
    }
}

/// The custom-property environment the engine named for a pseudo-element it settled, which the
/// pseudo-element takes once the host installs its record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NamedPseudoElementEnvironment {
    /// Zero for none.
    identity: u64,
    /// Whether it is not simply the environment the element passes on.
    declares: bool,
    /// Whether it is the environment the element holds, which the host resolved: the pseudo-element
    /// holds the host's object for it too.
    views_element_environment: bool,
    /// For the environment the pseudo-element's sample composed its animated custom properties
    /// into, the one its record was resolved over, beneath it.
    animation_base: Option<u64>,
    /// Whether the sample the engine took as it settled the pseudo-element moved it here, and the
    /// host installs nothing of the sample's.
    from_sample: bool,
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
    /// The custom-property environment the sample moved the element to, where it moved it.
    pub(crate) custom_properties: Option<SampledCustomPropertyEnvironment>,
    pub(crate) invalidation: bridge::FfiAnimationInvalidation,
    pub(crate) overlay_is_empty: bool,
    pub(crate) substitution_marks: u8,
    pub(crate) keyframes_inherited_non_inherited_style_groups: u32,
    pub(crate) uses_tree_counting_function: bool,
    /// Whether building the composition rebuilt every style group rather than the ones the overlay
    /// writes.
    pub(crate) rebuilt_every_group: bool,
}

/// The environment an element's animations composed its animated custom properties into, which the
/// engine minted over the environment its record was published with, for the host to view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SampledCustomPropertyEnvironment {
    /// The engine's identity for it; zero where the sample animated no custom property and the
    /// element holds its record's environment again.
    pub(crate) environment: u64,
    /// Whether the element's own style reads custom properties, and so has to be computed again
    /// under it; and whether a name its descendants inherit moved.
    pub(crate) element_reads: bool,
    pub(crate) inherited_names_moved: bool,
}

impl RetainedState {
    /// The environment an element's own declarations resolved to: the one it holds, or, where that
    /// is the one an earlier sample composed its animated custom properties into, the one beneath.
    pub(crate) fn element_base_custom_property_environment(&self, node: StyleNodeID) -> Result<u64, &'static str> {
        let environment = self
            .computed_group_sets
            .custom_property_environment_identity(node)
            .unwrap_or(0);
        if self.sampled_custom_property_environments.get(&node) != Some(&environment) {
            return Ok(environment);
        }
        // One the host composed names its base when the element installs it.
        self.custom_property_environments
            .engine_environment(environment)
            .map(|(_, base)| base)
            .or_else(|| {
                self.element_custom_property_animation_base(node)
                    .map(|(base, _, _)| base)
            })
            .ok_or("a sampled environment with no base")
    }

    /// Compose what a sample animated of an element's custom properties into an environment of the
    /// engine's own, over `base_environment`, the one the element's record was published with, and
    /// make it the element's: the environment its own values substitute under and its children
    /// inherit. `None` where that leaves the element's environment as it is.
    pub(crate) fn publish_sampled_custom_properties(
        &mut self,
        node: StyleNodeID,
        base_environment: u64,
        animated: &[(
            crate::css::retained_fly_string::RetainedUtf16FlyString,
            crate::css::style_value::RetainedStyleValueData,
        )],
    ) -> Result<Option<SampledCustomPropertyEnvironment>, &'static str> {
        use crate::css::custom_properties::CustomPropertyStore;

        let base_store = match base_environment {
            0 => std::ptr::null(),
            environment => self
                .custom_property_environments
                .store(environment)
                .ok_or("a base environment without a store")?,
        };
        let sampled = self.sampled_custom_property_environments.get(&node).copied();
        // A sample that animates what the element's environment already holds, over the same base,
        // moves nothing. A record computed again publishes its environment under a new identity even
        // where it resolves as before, which moves nothing either: reacting to it would compute the
        // element again, and so on without end. The host is only handed the environment again where
        // it installed another one meanwhile.
        if let Some(sampled) = sampled
            && !animated.is_empty()
            && self.sampled_environment_is_over(sampled, base_environment)
            && self
                .custom_property_environments
                .store(sampled)
                .is_some_and(|store| unsafe { CustomPropertyStore::composes_exactly(store, animated) })
        {
            self.computed_group_sets
                .set_node_custom_property_environment(node, sampled);
            if self.element_custom_property_data(node).1 == sampled {
                return Ok(None);
            }
            return Ok(Some(SampledCustomPropertyEnvironment {
                environment: sampled,
                element_reads: false,
                inherited_names_moved: false,
            }));
        }
        if sampled.is_none() && animated.is_empty() {
            return Ok(None);
        }
        let environment = match animated.is_empty() {
            true => {
                self.sampled_custom_property_environments.remove(&node);
                0
            }
            false => {
                let store = unsafe { CustomPropertyStore::animation_overlay_over(base_store, animated) };
                let environment = unsafe {
                    self.custom_property_environments
                        .adopt_engine_environment(store, base_environment)
                };
                self.sampled_custom_property_environments.insert(node, environment);
                environment
            }
        };
        self.computed_group_sets.set_node_custom_property_environment(
            node,
            match environment {
                0 => base_environment,
                environment => environment,
            },
        );
        // Taking the overlay away can expose any inherited value it covered.
        let inherited_names_moved = animated.is_empty() || {
            // SAFETY: The document owns the published registry for the length of the pass.
            let registry = unsafe {
                self.document_style_computation_inputs
                    .custom_property_registry
                    .as_pointer()
                    .cast::<crate::css::custom_properties::CustomPropertyRegistry>()
                    .as_ref()
            };
            animated.iter().any(|(name, _)| {
                let text: Vec<u16> = match unsafe { ak::utf16_string_units(name.raw_word()) } {
                    ak::Utf16StringUnits::Ascii(bytes) => bytes.iter().map(|&unit| u16::from(unit)).collect(),
                    ak::Utf16StringUnits::Utf16(units) => units.to_vec(),
                };
                registry
                    .and_then(|registry| registry.registration_facts(&text))
                    .is_none_or(|registration| registration.inherits)
            })
        };
        Ok(Some(SampledCustomPropertyEnvironment {
            environment,
            element_reads: self.node_style_reads_custom_properties(node),
            inherited_names_moved,
        }))
    }

    /// Compose what a sample animated of a synthetic pseudo-element's custom properties into an
    /// environment of the engine's own over `base_environment`, the one its record was published
    /// with, for the host to view as the pseudo-element's. A pseudo-element has no descendants, and
    /// its element computes again under it, as under one the host composed. `None` where that leaves
    /// the pseudo-element's environment as it is.
    pub(crate) fn publish_sampled_pseudo_element_custom_properties(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        base_environment: u64,
        animated: &[(
            crate::css::retained_fly_string::RetainedUtf16FlyString,
            crate::css::style_value::RetainedStyleValueData,
        )],
    ) -> Result<Option<SampledCustomPropertyEnvironment>, &'static str> {
        use crate::css::custom_properties::CustomPropertyStore;

        let base_store = match base_environment {
            0 => std::ptr::null(),
            environment => self
                .custom_property_environments
                .store(environment)
                .ok_or("a base environment without a store")?,
        };
        let key = (node, pseudo_kind);
        let sampled = self
            .sampled_pseudo_element_custom_property_environments
            .get(&key)
            .copied();
        let held = self
            .pseudo_element_custom_property_data
            .get(&key)
            .map(|held| held.identity);
        // A sample that animates what the environment already holds, over the same base, moves
        // nothing the pseudo-element does not hold already.
        if let Some(sampled) = sampled
            && !animated.is_empty()
            && self.sampled_environment_is_over(sampled, base_environment)
            && self
                .custom_property_environments
                .store(sampled)
                .is_some_and(|store| unsafe { CustomPropertyStore::composes_exactly(store, animated) })
        {
            if held == Some(sampled) {
                return Ok(None);
            }
            return Ok(Some(SampledCustomPropertyEnvironment {
                environment: sampled,
                element_reads: true,
                inherited_names_moved: false,
            }));
        }
        // Nothing to compose, over a pseudo-element holding no composition, moves nothing either.
        let holds_composition = sampled.is_some()
            || self
                .pseudo_element_custom_property_data
                .get(&key)
                .is_some_and(|held| held.animation_base.is_some());
        if animated.is_empty() && !holds_composition {
            return Ok(None);
        }
        let environment = match animated.is_empty() {
            true => {
                self.sampled_pseudo_element_custom_property_environments.remove(&key);
                0
            }
            false => {
                let store = unsafe { CustomPropertyStore::animation_overlay_over(base_store, animated) };
                let environment = unsafe {
                    self.custom_property_environments
                        .adopt_engine_environment(store, base_environment)
                };
                self.sampled_pseudo_element_custom_property_environments
                    .insert(key, environment);
                environment
            }
        };
        Ok(Some(SampledCustomPropertyEnvironment {
            environment,
            element_reads: true,
            inherited_names_moved: false,
        }))
    }
}

impl super::StyleEngineState {
    /// The record the host holds for an element, as it last installed one.
    pub(crate) fn held_style_record(&self, node: StyleNodeID) -> Option<u64> {
        self.host.held_style_records.get(&node).copied()
    }

    /// Build the payloads of the overlay record a sample composed over its record, as the host's
    /// publication builds them: the groups the overlay writes rebuilt against the record's, with
    /// the used color scheme and the display before the box-type transformation the overlay
    /// leaves, and the font it asks for resolved.
    pub(crate) fn build_settled_row_payloads(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        sample: &crate::css::style_compute::SettledRowSample,
        counters: &mut super::Counters,
    ) -> Result<super::animations::AnimationOverlayPayloads, &'static str> {
        use crate::css::computed_value_views::ComputedValuesView;
        use crate::css::host_shared::SharedPayload;
        use crate::css::property_metadata::property_id;

        let style_record = sample.style_record;
        let overlay = unsafe { &*sample.style.overlay };
        let table = unsafe { &*sample.style.table };
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
            let scheme = match overlay.get(property_id::COLOR_SCHEME) {
                Some(_) => crate::css::style_compute::animated_used_color_scheme(
                    table,
                    overlay,
                    &self.retained.document_style_computation_inputs,
                ),
                None => u8::try_from(scheme).map_err(|_| "a record with no effective color scheme")?,
            };
            (
                scheme,
                sample
                    .animated_display_before_box_type_transformation
                    .unwrap_or(display),
            )
        };
        let font_unresolved = std::cell::Cell::new(false);
        let mut refilled_font = false;
        let payloads = loop {
            let retained = &self.retained;
            let mut font = || {
                let inputs = retained.animated_font_group_inputs(node, table, Some(overlay));
                font_unresolved.set(inputs.is_none());
                inputs
            };
            let payloads = unsafe {
                self.build_animation_overlay_payloads(
                    node,
                    pseudo_kind,
                    style_record,
                    table,
                    Some(overlay),
                    used_color_scheme,
                    display_before_box_type_transformation,
                    &mut font,
                )
            };
            match payloads {
                Some(payloads) => break payloads,
                // A font the overlay asks for that nobody resolved yet is resolved between two
                // complete builds, as a parked row's font is, and the overlay is built again.
                None if font_unresolved.get() && !refilled_font && self.retained.font_resolution.is_some() => {
                    let request = font_resolution_inputs(
                        table,
                        Some(overlay),
                        self.retained.tree.tree_scope(node).0,
                        &self.retained.document_style_computation_inputs,
                    )
                    .request;
                    self.refill_font_requests(
                        vec![(Some(node), super::font_resolution::FontRequest::new(request))],
                        counters,
                    );
                    refilled_font = true;
                }
                None if font_unresolved.get() => return Err("an overlay font nobody resolved yet"),
                None => return Err("no record to compose over"),
            }
        };
        Ok(payloads)
    }

    /// Publish what the pass's sample of a settled row composed as the element's overlay record,
    /// which the rows after it inherit from, and keep what the host applies when it installs the
    /// row. Or why the engine cannot, where the host samples the row itself.
    pub(crate) fn publish_settled_row_sample(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        sample: crate::css::style_compute::SettledRowSample,
        counters: &mut super::Counters,
    ) -> Result<SettledRowPublication, &'static str> {
        use crate::css::host_shared::{HostShared, SharedPayload};

        let pseudo_kind = pseudo.unwrap_or(u8::MAX);
        let style_record = sample.style_record;
        let overlay = unsafe { &*sample.style.overlay };
        let base_environment = match pseudo {
            None => self.retained.element_base_custom_property_environment(node)?,
            Some(_) => self
                .retained
                .computed_group_sets
                .style_record_custom_property_environment(style_record)
                .unwrap_or(0),
        };
        if base_environment != 0 && self.custom_property_environments.store(base_environment).is_none() {
            return Err("a base environment without a store");
        }
        let payloads = self.build_settled_row_payloads(node, pseudo_kind, &sample, counters)?;
        let rebuilt_every_group = payloads.rebuilt_every_group;
        let shared = SharedPayload::from_pointer_slice(&payloads.payloads);
        let is_document_element = pseudo.is_none()
            && self.computed_group_sets.adjustment_facts(node) & element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0;
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
                computed::ComputedStyleTarget::new(node, pseudo_kind),
                identity,
                HostShared::new(sample.style.overlay.cast_const()),
                if overlay_is_empty { &[] } else { shared },
                counters,
            )
            .ok_or("no overlay publication")?;
        let style_record = publication.style_record.raw();
        if pseudo.is_none() {
            self.retained
                .computed_group_sets
                .set_sampled_composition_identity(node, style_record);
        }
        // A sample the host publishes before it installs the row replaces the composition in its
        // slot, so the batch keeps it alive for the row until the host acknowledges it.
        if !overlay_is_empty {
            self.retained.computed_group_sets.pin_style_record(style_record);
            self.retained.batch_pinned_compositions.push((node, style_record));
        }
        let custom_properties = match pseudo {
            None => self.retained.publish_sampled_custom_properties(
                node,
                base_environment,
                &sample.animated_custom_properties,
            )?,
            Some(kind) => self.retained.publish_sampled_pseudo_element_custom_properties(
                node,
                kind,
                base_environment,
                &sample.animated_custom_properties,
            )?,
        };
        let published = SettledRowPublication {
            style_record,
            custom_properties,
            invalidation,
            overlay_is_empty,
            substitution_marks: sample.substitution_marks,
            keyframes_inherited_non_inherited_style_groups: sample.keyframes_inherited_non_inherited_style_groups,
            uses_tree_counting_function: sample.uses_tree_counting_function,
            rebuilt_every_group,
        };
        match pseudo {
            None => self.retained.rows_sampled_in_pass.insert(node, published),
            Some(kind) => self
                .retained
                .pseudo_elements_sampled_in_pass
                .insert((node, kind), published),
        };
        Ok(published)
    }

    /// Take what the pass published for a row whose animations it sampled, so that exactly one
    /// installation applies it.
    pub(crate) fn take_row_sampled_in_pass(&mut self, node: StyleNodeID) -> Option<SettledRowPublication> {
        self.retained.rows_sampled_in_pass.remove(&node)
    }

    /// Sample the animations of the synthetic pseudo-elements the engine just settled for an
    /// element over its composition, and decide their transition steps over what the sample
    /// composed, as the host would once it installs them over `held_style_records`, the records it
    /// holds for them: publish each composition as the pseudo-element's record and name it in the
    /// answer in place of the settled one, so the host installs it and applies the step's decisions
    /// rather than sampling and deciding itself. Returns the kinds whose sample the engine took,
    /// and the kinds whose step it decided; the host samples and steps the rest.
    pub(crate) fn sample_settled_pseudo_elements(
        &mut self,
        node: StyleNodeID,
        settled: &mut super::publication::RetriedEngineRecord,
        held_style_records: &[u64; bridge::RETRY_PSEUDO_RECORD_SLOTS],
        committed_boxes: super::animations::CommittedTransformReferenceBoxes,
        timeline_samples: &super::animations::AnimationTimelineSamples,
        counters: &mut super::Counters,
    ) -> (u8, u8) {
        let mut sampled = 0u8;
        let mut stepped = 0u8;
        for (kind, &held_style_record) in held_style_records.iter().enumerate() {
            // A kind the settle does not name keeps the record it has, and the host samples it.
            if (settled.pseudo_records_present >> kind) & 1 == 0 {
                continue;
            }
            let pseudo_kind = kind as u8;
            let record = settled.pseudo_records[kind];
            // A pseudo-element that generates no box has nothing to compose or transition.
            if record == 0 {
                sampled |= 1 << kind;
                continue;
            }
            if self.assigned_style_record_of(node, Some(pseudo_kind)) != Some(record) {
                engine_sample_check::note_declined("pseudo-element: a record the engine has not assigned");
                continue;
            }
            let slot = crate::css::style_compute::animation_slot(pseudo_kind);
            let installed = match self.retained.element_animation_timing_rows(node, slot).is_empty()
                && !self.retained.element_has_animation_effect_descriptions(node, slot)
            {
                // One with no effect to sample composes nothing over its record.
                true => record,
                false => {
                    let published = crate::css::style_compute::sample_settled_row(
                        self,
                        node,
                        Some(pseudo_kind),
                        false,
                        None,
                        None,
                        committed_boxes,
                        timeline_samples,
                    )
                    .and_then(|sample| {
                        self.publish_settled_row_sample(node, Some(pseudo_kind), sample, counters)
                            .map_err(String::from)
                    });
                    match published {
                        Ok(published) => {
                            engine_sample_check::note_taken("pseudo-element sample");
                            published.style_record
                        }
                        Err(reason) => {
                            engine_sample_check::note_declined(&format!("pseudo-element: {reason}"));
                            continue;
                        }
                    }
                }
            };
            sampled |= 1 << kind;
            settled.pseudo_records[kind] = installed;
            if let Some(composition) = self.decide_settled_pseudo_element_transition_step(
                node,
                pseudo_kind,
                held_style_record,
                record,
                installed,
                committed_boxes,
                timeline_samples,
                counters,
            ) {
                stepped |= 1 << kind;
                settled.pseudo_records[kind] = composition;
            }
        }
        (sampled, stepped)
    }

    /// Sample the animations of an element or one of its pseudo-elements over the record the host
    /// holds for it, from the timing rows, descriptions and environments the engine holds, and
    /// publish the composition as its record, for the host to install in place of its own sample.
    /// Where the sample moves nothing the record composed, the answer names the record again with
    /// what the sample found out; or why the engine cannot.
    pub(crate) fn sample_installed_record(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        style_record: u64,
        committed_boxes: super::animations::CommittedTransformReferenceBoxes,
        timeline_samples: &super::animations::AnimationTimelineSamples,
        counters: &mut super::Counters,
    ) -> Result<SettledRowPublication, String> {
        // The effects the element holds now are sampled, as the host's own sample collects them,
        // whether or not a plan its row left is applied yet.
        let sample = crate::css::style_compute::sample_settled_row(
            self,
            node,
            pseudo,
            false,
            Some(style_record),
            None,
            committed_boxes,
            timeline_samples,
        )?;
        // A sample that moves nothing the record composed may still move the custom properties
        // the element's environment animates, which the publication composes.
        // A composition the engine published over the record's base since, which the record does
        // not name, is published again even where the values have not moved.
        let assigned = self.assigned_style_record_of(node, pseudo);
        let engine_holds_a_stale_composition = assigned.is_some_and(|assigned| {
            assigned != style_record && self.computed_group_sets.base_style_record_of(assigned) == style_record
        });
        let holds_sampled_custom_properties = match pseudo {
            None => self.retained.sampled_custom_property_environments.contains_key(&node),
            Some(kind) => self
                .retained
                .sampled_pseudo_element_custom_property_environments
                .contains_key(&(node, kind)),
        };
        if sample.animated_custom_properties.is_empty()
            && !holds_sampled_custom_properties
            && !engine_holds_a_stale_composition
            && !self.animation_overlay_changed(style_record, sample.style.overlay)
        {
            return Ok(SettledRowPublication {
                style_record,
                custom_properties: None,
                invalidation: bridge::FfiAnimationInvalidation::default(),
                overlay_is_empty: unsafe { sample.style.overlay.as_ref() }.is_none_or(|overlay| overlay.is_empty()),
                substitution_marks: sample.substitution_marks,
                keyframes_inherited_non_inherited_style_groups: sample.keyframes_inherited_non_inherited_style_groups,
                uses_tree_counting_function: sample.uses_tree_counting_function,
                rebuilt_every_group: false,
            });
        }
        let published = self.publish_settled_row_sample(node, pseudo, sample, counters)?;
        // The host installs the composition as it returns, so no batch keeps it alive for a row.
        if let Some(index) = self
            .retained
            .batch_pinned_compositions
            .iter()
            .rposition(|&pinned| pinned == (node, published.style_record))
        {
            self.retained.batch_pinned_compositions.swap_remove(index);
            self.retained
                .computed_group_sets
                .unpin_style_record(published.style_record);
        }
        match pseudo {
            None => self.retained.rows_sampled_in_pass.remove(&node),
            Some(kind) => self.retained.pseudo_elements_sampled_in_pass.remove(&(node, kind)),
        };
        Ok(published)
    }

    /// Forget what the engine sampled and decided for an element's pseudo-elements that no
    /// installation applied, before it settles them again.
    pub(crate) fn forget_pseudo_elements_sampled_in_pass(&mut self, node: StyleNodeID) {
        self.retained
            .pseudo_elements_sampled_in_pass
            .retain(|(owner, _), _| *owner != node);
        self.retained
            .pseudo_element_transition_steps_decided_in_pass
            .retain(|(owner, _), _| *owner != node);
        self.retained
            .pseudo_element_environments_named_in_settle
            .retain(|(owner, _), _| *owner != node);
    }

    /// The custom-property environment a settled pseudo-element record was resolved over, which the
    /// pseudo-element holds once its record is installed, named by the engine itself rather than
    /// installed by the host: an environment the engine resolved, by identity, or the environment
    /// its element passes on. A pseudo-element declares custom properties of its own where that is
    /// neither the element's environment nor its inherited projection.
    ///
    /// The pseudo-element takes it once the host installs the record. The host names it where the
    /// engine does not: for a pseudo-element holding an animation overlay, which a new base
    /// re-layers, and for one inheriting an environment the host resolved that its element passes
    /// on as a projection of its own.
    pub(crate) fn name_settled_pseudo_element_environment(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        style_record: u64,
    ) {
        match self.settled_pseudo_element_environment(node, pseudo_kind, style_record) {
            Some(named) => {
                self.retained
                    .pseudo_element_environments_named_in_settle
                    .insert((node, pseudo_kind), named);
            }
            None => engine_sample_check::note_declined("pseudo-element environment: named by the host"),
        }
    }

    fn settled_pseudo_element_environment(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        style_record: u64,
    ) -> Option<NamedPseudoElementEnvironment> {
        let key = (node, pseudo_kind);
        // What the sample the engine took as it settled the pseudo-element composed its animated
        // custom properties into, where it moved them.
        let sampled = self
            .retained
            .pseudo_elements_sampled_in_pass
            .get(&key)
            .and_then(|published| published.custom_properties)
            .map(|moved| moved.environment);
        // An animation overlay the sample did not move is re-layered over a new base by the host.
        if sampled.is_none()
            && self
                .retained
                .pseudo_element_custom_property_data
                .get(&key)
                .is_some_and(|held| held.is_animation_overlay)
        {
            return None;
        }
        let environment = self
            .retained
            .computed_group_sets
            .style_record_custom_property_environment(style_record)?;
        let element_held = self
            .retained
            .element_custom_property_data
            .get(&node)
            .and_then(Option::as_ref);
        let element_environment = element_held.map_or(0, |held| held.identity);
        let named = match environment {
            0 => NamedPseudoElementEnvironment {
                identity: 0,
                declares: false,
                views_element_environment: false,
                animation_base: None,
                from_sample: false,
            },
            environment if environment & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT != 0 => {
                let inputs = self.retained.document_style_computation_inputs;
                let inherited = match self.retained.custom_property_environments.store(element_environment) {
                    Some(_) => self
                        .retained
                        .inheritable_custom_property_environment(element_environment, &inputs),
                    None => element_environment,
                };
                NamedPseudoElementEnvironment {
                    identity: environment,
                    declares: environment != element_environment && environment != inherited,
                    views_element_environment: false,
                    animation_base: None,
                    from_sample: false,
                }
            }
            // One the host resolved is what the element passes on: its own environment, where no
            // registration keeps a name of it from inheriting.
            _ => {
                let element_held = element_held.filter(|held| held.data.is_some())?;
                let inputs = self.retained.document_style_computation_inputs;
                let identity = element_held.identity;
                if self.retained.custom_property_environments.store(identity).is_some()
                    && self.retained.inheritable_custom_property_environment(identity, &inputs) != identity
                {
                    return None;
                }
                NamedPseudoElementEnvironment {
                    identity,
                    declares: false,
                    views_element_environment: true,
                    animation_base: None,
                    from_sample: false,
                }
            }
        };
        match sampled {
            None => Some(named),
            Some(0) => Some(NamedPseudoElementEnvironment {
                from_sample: true,
                ..named
            }),
            // The engine views what the sample composed over an environment it resolved itself.
            Some(sampled) => {
                if named.views_element_environment || !self.sampled_environment_is_over(sampled, named.identity) {
                    return None;
                }
                Some(NamedPseudoElementEnvironment {
                    identity: sampled,
                    declares: named.declares,
                    views_element_environment: false,
                    animation_base: Some(named.identity),
                    from_sample: true,
                })
            }
        }
    }

    /// Whether the engine named the environment of a pseudo-element it settled from the sample it
    /// took of it, so that the host installs nothing of the sample's.
    pub(crate) fn pseudo_element_environment_named_from_sample(&self, node: StyleNodeID, pseudo_kind: u8) -> bool {
        self.retained
            .pseudo_element_environments_named_in_settle
            .get(&(node, pseudo_kind))
            .is_some_and(|named| named.from_sample)
    }

    /// The host installs the record of a pseudo-element the engine settled: it takes the
    /// environment the engine named for it, if any. Returns whether it did, and the host installs
    /// none.
    pub(crate) fn take_pseudo_element_environment_named_in_settle(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> bool {
        let key = (node, pseudo_kind);
        let Some(named) = self.retained.pseudo_element_environments_named_in_settle.remove(&key) else {
            return false;
        };
        // The host's object for an environment it resolved is the one the element holds.
        let data = match named {
            named if named.views_element_environment => {
                let Some(data) = self
                    .retained
                    .element_custom_property_data
                    .get(&node)
                    .and_then(Option::as_ref)
                    .filter(|held| held.identity == named.identity)
                    .and_then(|held| held.data.as_ref())
                    .map(inputs::RetainedCustomPropertyData::share)
                else {
                    return false;
                };
                Some(data)
            }
            _ => None,
        };
        let retired = match named.identity {
            0 => self.retained.pseudo_element_custom_property_data.remove(&key),
            identity => {
                let animation_base = named.animation_base.map(|base| {
                    inputs::AnimationBaseEnvironment::resolved_by_engine(
                        base,
                        self.retained
                            .custom_property_environments
                            .store(base)
                            .unwrap_or(std::ptr::null()),
                    )
                });
                self.retained.pseudo_element_custom_property_data.insert(
                    key,
                    inputs::HeldCustomPropertyEnvironment {
                        identity,
                        is_animation_overlay: animation_base.is_some(),
                        declares: named.declares,
                        data,
                        animation_base,
                    },
                )
            }
        };
        self.host
            .retired_custom_property_data
            .extend(retired.and_then(|held| held.data));
        engine_sample_check::note_taken(match (named.from_sample, named.animation_base.is_some()) {
            (true, true) => "pseudo-element environment named in settle: sampled overlay",
            (true, false) => "pseudo-element environment named in settle: sample cleared the overlay",
            (false, _) => "pseudo-element environment named in settle: record environment",
        });
        true
    }

    /// A sample of a synthetic pseudo-element over the record the host holds moved its
    /// custom-property environment: to `sampled`, the one the engine composed its animated custom
    /// properties into, or, with zero, back to the one beneath. The pseudo-element takes it here,
    /// where the host would have installed its view of it, and the host views it by identity.
    ///
    /// Returns false where the host installs it: over an environment the host resolved, whose
    /// object the engine does not hold.
    pub(crate) fn install_sampled_pseudo_element_environment(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        sampled: u64,
    ) -> bool {
        let key = (node, pseudo_kind);
        let (base, declares) = match self.retained.pseudo_element_custom_property_data.get(&key) {
            None => (0, false),
            Some(held) if held.is_animation_overlay => match held.animation_base.as_ref() {
                Some(base) => (base.environment(), held.declares),
                None => return false,
            },
            Some(held) => (held.identity, held.declares),
        };
        if base != 0 && base & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT == 0 {
            engine_sample_check::note_declined("pseudo-element sampled environment: over a host environment");
            return false;
        }
        let retired = match (sampled, base) {
            (0, 0) => self.retained.pseudo_element_custom_property_data.remove(&key),
            (0, base) => self.retained.pseudo_element_custom_property_data.insert(
                key,
                inputs::HeldCustomPropertyEnvironment {
                    identity: base,
                    is_animation_overlay: false,
                    declares,
                    data: None,
                    animation_base: None,
                },
            ),
            (sampled, base) => {
                let base_store = self
                    .retained
                    .custom_property_environments
                    .store(base)
                    .unwrap_or(std::ptr::null());
                self.retained.pseudo_element_custom_property_data.insert(
                    key,
                    inputs::HeldCustomPropertyEnvironment {
                        identity: sampled,
                        is_animation_overlay: true,
                        declares,
                        data: None,
                        animation_base: Some(inputs::AnimationBaseEnvironment::resolved_by_engine(base, base_store)),
                    },
                )
            }
        };
        self.host
            .retired_custom_property_data
            .extend(retired.and_then(|held| held.data));
        engine_sample_check::note_taken("pseudo-element sampled environment installed by the engine");
        true
    }

    /// A sample of an element moved its custom-property environment: to `sampled`, the one the
    /// engine composed its animated custom properties into, or, with zero, back to the one beneath.
    /// The element takes it here, where the host would have installed its view of it, and the host
    /// views it by identity.
    ///
    /// Returns false where the host installs it: over an environment the host resolved, whose
    /// object the engine does not hold.
    pub(crate) fn install_sampled_element_environment(&mut self, node: StyleNodeID, sampled: u64) -> bool {
        let (base, declares) = match self.retained.element_custom_property_data.get(&node) {
            None | Some(None) => (0, false),
            Some(Some(held)) if held.is_animation_overlay => match held.animation_base.as_ref() {
                Some(base) => (base.environment(), held.declares),
                None => return false,
            },
            Some(Some(held)) => (held.identity, held.declares),
        };
        if base != 0 && base & custom_property_environments::ENGINE_ENVIRONMENT_IDENTITY_BIT == 0 {
            engine_sample_check::note_declined("element sampled environment: over a host environment");
            return false;
        }
        let held = match (sampled, base) {
            (0, 0) => None,
            (0, base) => Some(inputs::HeldCustomPropertyEnvironment {
                identity: base,
                is_animation_overlay: false,
                declares,
                data: None,
                animation_base: None,
            }),
            (sampled, base) => {
                let base_store = self
                    .retained
                    .custom_property_environments
                    .store(base)
                    .unwrap_or(std::ptr::null());
                Some(inputs::HeldCustomPropertyEnvironment {
                    identity: sampled,
                    is_animation_overlay: true,
                    declares,
                    data: None,
                    animation_base: Some(inputs::AnimationBaseEnvironment::resolved_by_engine(base, base_store)),
                })
            }
        };
        if let Some(held) = held.as_ref() {
            self.retained
                .computed_group_sets
                .set_node_custom_property_environment(node, held.identity);
        }
        let retired = self.retained.element_custom_property_data.insert(node, held);
        self.host
            .retired_custom_property_data
            .extend(retired.flatten().and_then(|held| held.data));
        engine_sample_check::note_taken("element sampled environment installed by the engine");
        true
    }

    /// Take what the engine published for a pseudo-element whose animations it sampled as it
    /// settled it, so that exactly one installation applies it.
    pub(crate) fn take_pseudo_element_sampled_in_pass(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<SettledRowPublication> {
        self.retained
            .pseudo_elements_sampled_in_pass
            .remove(&(node, pseudo_kind))
    }
}
