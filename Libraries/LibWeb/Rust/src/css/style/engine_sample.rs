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
use crate::css::computed_value_types::{FontValues, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
use crate::css::computed_values::InheritedBoxValues;
use crate::css::style_compute::{FfiAnimationLengthContexts, FfiFontMetrics, FfiLengthResolutionContext};

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
