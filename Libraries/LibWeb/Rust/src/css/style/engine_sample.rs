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

    /// The three length-resolution contexts a sample of the element's animations computes keyframe
    /// values in, over the record the element holds: the font context, which reads the element's
    /// inheritance parent; the line-height context, which reads the element's own font and the
    /// parent's line height; and the one everything else resolves against, the element's own font.
    /// A mirror of the host's `get_computation_context_for_property(FontFamily / LineHeight /
    /// Color)` over the working set it reconstructs from that record, with the container bases
    /// `container_unit_mask` asks for.
    ///
    /// `None` where the element holds no record the engine can read.
    pub(crate) fn animation_sample_length_contexts(
        &self,
        node: StyleNodeID,
        pseudo_kind: Option<u8>,
        style_record: u64,
        container_unit_mask: u8,
    ) -> Option<FfiAnimationLengthContexts> {
        let inputs: &bridge::FfiDocumentStyleComputationInputs = &self.document_style_computation_inputs;
        let own = self.record_font(style_record)?;
        // A pseudo-element inherits from its originating element.
        let parent = match pseudo_kind {
            Some(_) => Some(node),
            None => self.tree.inheritance_parent(node),
        };
        let parent_record = parent
            .and_then(|parent| self.computed_group_sets.assigned_style_record(parent))
            .map(computed::FinalStyleRecordID::raw);
        let parent_font = parent_record.and_then(|record| self.record_font(record));
        let is_document_element =
            self.computed_group_sets.adjustment_facts(node) & element_adjustment_fact::IS_DOCUMENT_ELEMENT != 0;
        let subject_inline_axis_is_horizontal = own.inline_axis_is_horizontal;
        let (width_basis, height_basis) =
            self.container_unit_bases(node, container_unit_mask, subject_inline_axis_is_horizontal, inputs);
        // What a `rem` resolves against is the font of the record the host last installed on the
        // document element, which is what the host's own member holds - not the document inputs,
        // which were published before this update installed anything.
        let root = self.root_element_font_metrics;
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

/// A description of font group inputs two resolutions can be compared by.
pub(crate) fn describe_font_group_build_inputs(inputs: &FfiFontGroupBuildInputs) -> String {
    format!(
        "size {} line height {} emoji {} ascent {} descent {} x-height {} zero {} first {:p} list {:p} weight {} width {} math {}/{}/{}",
        inputs.font_size_raw,
        inputs.line_height_used_raw,
        inputs.font_variant_emoji,
        inputs.font_ascent,
        inputs.font_descent,
        inputs.font_x_height,
        inputs.font_zero_advance,
        inputs.first_available_font,
        inputs.font_cascade_list,
        inputs.font_weight,
        inputs.font_width,
        inputs.math_shift,
        inputs.math_style,
        inputs.math_depth,
    )
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
        // The engine holds no pseudo-element's environment.
        if pseudo_kind.is_some() {
            return Err("a pseudo-element");
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
        // The base is what the element inherits where it is the environment the parent passes on.
        let inputs = self.document_style_computation_inputs;
        // Where custom properties are registered, the host builds the environment a parent passes
        // on as an object of its own - a projection without the names that do not inherit, over the
        // projection of its own parent - which the engine cannot name.
        let registry = unsafe {
            inputs
                .custom_property_registry
                .as_pointer()
                .cast::<crate::css::custom_properties::CustomPropertyRegistry>()
                .as_ref()
        };
        let Some(registry) = registry else {
            return Err("a document that published no registry");
        };
        if registry.has_registrations() {
            return Err("a parent whose inheritable environment the host builds");
        }
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
}
