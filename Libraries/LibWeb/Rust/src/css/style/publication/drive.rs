/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;
use crate::css::cascaded_properties::CascadedValues;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FontDriveGoal {
    Complete,
    RootInputs,
}

/// Why a row has no record. A suspended row is not refused: it waits on a request the host
/// services between passes, and the same row resumes once that request is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::css::style) enum Unanswered {
    Suspended(Suspension),
    Refused,
}

/// What a suspended row waits on. The request itself stays where the host's service reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::css::style) enum Suspension {
    /// The drive's font is not resolved yet; `FontDriveScratch::request` names it.
    Font,
    /// A `random()` base is not known yet; `RetainedState::random_base_requests` names it.
    RandomBases,
}

/// A row's answer from the engine, or why there is none.
pub(in crate::css::style) type Drive<T> = Result<T, Unanswered>;

/// What the monospace font-size recascade answers for a drive subject.
pub(super) enum MonospaceRecascade {
    /// The font size the recascade reaches, and whether reaching it read the viewport.
    Size(i32, bool),
    /// A length in the ancestor chain resolves against the monospace font at the size reached so
    /// far, which the between-pass font service has not resolved yet.
    AwaitsFont(bridge::FfiFontResolutionRequest),
}

/// A missing value on the way to a record is the engine declining the row.
pub(in crate::css::style) trait OrRefused<T> {
    fn or_refused(self) -> Drive<T>;
}

impl<T> OrRefused<T> for Option<T> {
    fn or_refused(self) -> Drive<T> {
        self.ok_or(Unanswered::Refused)
    }
}

/// A refusal counts under the cause the caller names; a suspension counts nowhere.
pub(in crate::css::style) trait CountRefusal {
    fn count_refusal(self, counters: &mut Counters, counter: Counter) -> Self;
}

impl<T> CountRefusal for Drive<T> {
    fn count_refusal(self, counters: &mut Counters, counter: Counter) -> Self {
        if let Err(Unanswered::Refused) = self {
            counters.bump(counter);
        }
        self
    }
}

/// A driven longhand table with the length context it was driven against, the longhand
/// evaluations it took, and the font a full drive resolved.
pub(super) type DrivenTable = (
    ComputedLonghandTable,
    crate::css::style_compute::FfiLengthResolutionContext,
    u32,
    Option<crate::css::table_group_builder::FfiFontGroupBuildInputs>,
);

/// What a partial drive answers besides a refusal.
#[expect(
    clippy::large_enum_variant,
    reason = "the driven table moves by value, as it did in an `Option`"
)]
pub(super) enum PartialDrive {
    Driven(DrivenTable),
    /// An input the drive reads for properties it did not select moved with the selection, or the
    /// old record holds no table to copy them from: the caller drives the record in full instead.
    DriverInputMoved,
}

/// What a full drive answers besides a refusal or a suspension.
#[expect(
    clippy::large_enum_variant,
    reason = "the driven table moves by value, as it did in an `Option`"
)]
pub(super) enum FullDrive {
    Driven(DrivenTable),
    /// The font phase is done and the registered custom properties resolve against this context
    /// before the drive resumes; the caller resumes it with the final store.
    AwaitsRegisteredContext(custom_property_cascade::RegisteredValueContext),
    /// The root-input probe finished the font and line-height phases; the drive is left pending
    /// for the root's own row.
    RootInputs(RootFontInputs),
}

#[derive(Default)]
pub(in crate::css::style) struct FontDriveScratch {
    pub(in crate::css::style) request: Option<font_resolution::FontRequest>,
    pending: Option<PendingFontDrive>,
}

impl FontDriveScratch {
    pub(in crate::css::style) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the suspended drive is this element's own. A row resumes only its own drive: the
    /// one left behind by an element the record loop settled another way is not an answer to it.
    pub(in crate::css::style) fn is_pending_for(&self, node: StyleNodeID) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.target == computed::ComputedStyleTarget::new(node, u8::MAX))
    }

    /// The font request a drive suspended on.
    pub(in crate::css::style) fn take_suspended_request(&mut self) -> font_resolution::FontRequest {
        self.request
            .take()
            .expect("a drive suspended on a font names its request")
    }

    pub(in crate::css::style) fn capacity_bytes(&self) -> u64 {
        self.pending
            .as_ref()
            .map_or(0, |pending| pending.table.owned_capacity_bytes())
    }
}

/// The completed font phase owns its table. No parent/context borrow survives refill;
/// the caller resumes the same subject before evaluating any later canonical element.
struct PendingFontDrive {
    /// What the suspended drive belongs to. The record loop can settle that element another way
    /// before the retry comes, which leaves the drive behind; whoever is driven next must not
    /// take up someone else's table.
    target: computed::ComputedStyleTarget,
    root_font_complete: bool,
    registered_finalization_ready: bool,
    recascaded_font_size: Option<i32>,
    table: ComputedLonghandTable,
    results: crate::css::style_compute::FfiLonghandDriverResults,
    effective_color_scheme: i16,
    resolved_viewport_relative_length: bool,
}

impl RetainedState {
    fn container_unit_bases(
        &self,
        node: StyleNodeID,
        mask: u8,
        inline_axis_is_horizontal: bool,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> (
        Option<animations::ContainerUnitBasis>,
        Option<animations::ContainerUnitBasis>,
    ) {
        let (needs_width, needs_height) =
            crate::css::style_compute::container_relative_axes_needed(mask, inline_axis_is_horizontal);
        (
            needs_width.then(|| self.container_unit_basis(node, true, inputs.viewport_width)),
            needs_height.then(|| self.container_unit_basis(node, false, inputs.viewport_height)),
        )
    }

    /// The font size the monospace recascade gives a drive subject: the cascaded font-size of
    /// every ancestor it inherits from, root first, walked again from a 13px default. A length the
    /// walk cannot resolve from the document's inputs alone resolves against the monospace font at
    /// the size reached so far and the line height its ancestor inherits; one that still does not
    /// resolve is skipped, as a `calc()` is.
    pub(super) fn monospace_recascaded_font_size(
        &self,
        target: computed::ComputedStyleTarget,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> MonospaceRecascade {
        use crate::css::computed_value_types::{STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
        use crate::css::css_pixels::CssPixels;
        use crate::css::style_compute::{
            FfiFontMetrics, FfiLengthResolutionContext, FontSizeRecascadeDocumentInputs, FontSizeRecascadeStatus,
            recascade_font_size_batch,
        };

        let mut nodes = Vec::new();
        let mut ancestor = self.retained_inheritance_parent_node(target.node(), target.pseudo_kind());
        while let Some(node) = ancestor {
            nodes.push(node);
            ancestor = self.tree.inheritance_parent(node);
        }
        nodes.reverse();
        let views = nodes
            .iter()
            .map(|&node| {
                self.computed_group_sets
                    .assigned_style_record(node)
                    .and_then(|record| self.style_record_view(record.raw()))
            })
            .collect::<Vec<_>>();
        let value_at = |index: usize| {
            views[index]
                .as_ref()
                .and_then(|view| unsafe { view.longhand_table.as_ref() })
                .map_or(std::ptr::null(), ComputedLonghandTable::raw_cascaded_font_size)
        };
        let default_size = CssPixels::from_integer(13).raw_value();
        let document_inputs = FontSizeRecascadeDocumentInputs {
            root_font_size: inputs.root_font_size,
            root_font_metrics_depend_on_viewport_metrics: inputs.root_font_metrics_depend_on_viewport_metrics,
            viewport_width: inputs.viewport_width,
            viewport_height: inputs.viewport_height,
        };
        let mut batch = recascade_font_size_batch(
            nodes.len(),
            value_at,
            0,
            default_size,
            false,
            default_size,
            document_inputs,
            std::ptr::null(),
        );
        loop {
            if batch.status == FontSizeRecascadeStatus::Complete {
                return MonospaceRecascade::Size(batch.current_size_raw, batch.depends_on_viewport_metrics);
            }
            let index = batch.next_index;
            let request = bridge::FfiFontResolutionRequest {
                font_family: bridge::FfiHostHandle::from_pointer(self.monospace_font_family.pointer().cast()),
                tree_scope: self.tree.tree_scope(target.node()).0,
                font_feature_values: [bridge::FfiHostHandle::from_pointer(std::ptr::null());
                    bridge::FONT_RESOLUTION_FEATURE_INPUT_COUNT],
                font_size_raw: batch.current_size_raw,
                font_slope: 0,
                font_weight: 400.0,
                font_width: 100.0,
                font_optical_sizing: 0,
                font_environment_generation: inputs.font_environment_generation,
            };
            let Some(resolved) = self
                .font_resolution
                .as_ref()
                .and_then(|resolutions| resolutions.lookup(request))
            else {
                return MonospaceRecascade::AwaitsFont(request);
            };
            // The ancestor's own font is the one being recascaded; its line height is the one it
            // inherits, and the initial one is zero.
            let (line_height, inherited_font_metrics_depend_on_viewport_metrics) = index
                .checked_sub(1)
                .and_then(|parent| views[parent].as_ref())
                .map_or((0.0, false), |view| {
                    let font = unsafe {
                        view.payloads[STYLE_GROUP_INDEX_FONT]
                            .cast::<crate::css::computed_value_types::FontValues>()
                            .deref()
                    };
                    (font.line_height_used.to_double(), view.dependency_flags & (1 << 1) != 0)
                });
            let subject_inline_axis_is_horizontal = views[index].as_ref().is_none_or(|view| {
                let inherited_box = unsafe {
                    view.payloads[STYLE_GROUP_INDEX_INHERITED_BOX]
                        .cast::<crate::css::computed_values::InheritedBoxValues>()
                        .deref()
                };
                inherited_box.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB
            });
            let (width_basis, height_basis) =
                self.container_unit_bases(nodes[index], u8::MAX, subject_inline_axis_is_horizontal, inputs);
            let context = FfiLengthResolutionContext {
                viewport_width: inputs.viewport_width,
                viewport_height: inputs.viewport_height,
                font_metrics: FfiFontMetrics {
                    font_size: CssPixels::from_raw(batch.current_size_raw).to_double(),
                    x_height: drive_font_metric(resolved.x_height),
                    cap_height: drive_font_metric(resolved.ascent),
                    zero_advance: drive_font_metric(resolved.zero_advance),
                    line_height,
                },
                root_font_metrics: FfiFontMetrics {
                    font_size: inputs.root_font_size,
                    x_height: inputs.root_font_x_height,
                    cap_height: inputs.root_font_cap_height,
                    zero_advance: inputs.root_font_zero_advance,
                    line_height: inputs.root_line_height,
                },
                font_metrics_depend_on_viewport_metrics: batch.depends_on_viewport_metrics
                    || inherited_font_metrics_depend_on_viewport_metrics,
                root_font_metrics_depend_on_viewport_metrics: inputs.root_font_metrics_depend_on_viewport_metrics,
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
            let resumed = recascade_font_size_batch(
                nodes.len(),
                value_at,
                index,
                batch.current_size_raw,
                batch.depends_on_viewport_metrics,
                default_size,
                document_inputs,
                &raw const context,
            );
            batch = match resumed.status {
                FontSizeRecascadeStatus::NeedsCppLengthResolution => recascade_font_size_batch(
                    nodes.len(),
                    value_at,
                    index + 1,
                    batch.current_size_raw,
                    batch.depends_on_viewport_metrics,
                    default_size,
                    document_inputs,
                    std::ptr::null(),
                ),
                _ => resumed,
            };
        }
    }

    /// Run the drive's remaining phase for the selected longhands over a copy of the node's
    /// current table, against the record's own font metrics, the document's computation inputs
    /// and the parent's record. The required driver inputs recompute on every drive and their
    /// post-compute adjustments read element facts this context does not carry, so the table
    /// stands only when they came out exactly as before.
    #[allow(clippy::too_many_arguments)]
    /// Whether what an explicit `inherit` of a non-inherited property would read from the parent
    /// is the record the engine holds. The installer's sampled composition is an authoritative
    /// parent record, including its active animation or transition overlay; so is an installed
    /// record whose transitions cannot start in this update.
    fn parent_record_answers_explicit_inheritance(&self, parent: Option<StyleNodeID>) -> bool {
        let Some(parent) = parent else {
            return true;
        };
        // A sampled identity only survives when it still equals the assigned record. A base
        // record published after the transition step is authoritative just like an overlay. Where
        // a running transition supplies the value, the child's after-change style inherits the
        // parent's after-change value, which the transition decision reads from the parent.
        if self.computed_group_sets.sampled_composition_identity(parent).is_some() {
            return true;
        }
        let Some(record) = self.computed_group_sets.assigned_style_record(parent) else {
            return false;
        };
        let Some(view) = self.computed_group_sets.style_record_view(record.raw()) else {
            return false;
        };
        let Some(table) = (unsafe { view.longhand_table.as_ref() }) else {
            return false;
        };
        let transitioned = crate::css::style_compute::active_transition_longhands(table);
        if transitioned.is_empty() {
            return true;
        }
        // A parent that declares transitions starts them only in its own transition step. The
        // record stands when no step is left for it in this update and no transition runs in it.
        // A post-compute adjustment also marks its overlay entry as a transition's, so that it
        // wins over an important declaration; only a property the parent transitions runs one.
        self.nodes_owing_a_transition_registration.get(&parent) != Some(&false)
            && unsafe { view.animated_overlay.as_ref() }.is_none_or(|overlay| {
                !overlay
                    .entries()
                    .iter()
                    .any(|entry| entry.result_of_transition && transitioned.contains(&entry.property))
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn engine_driven_table(
        &mut self,
        node: StyleNodeID,
        old_style_record: computed::FinalStyleRecordID,
        store: &WinnerStore,
        selected: &[u64],
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        explicitly_inherited_groups: &mut u32,
        counters: &mut Counters,
    ) -> Drive<PartialDrive> {
        let random_base_values = store.drive_random_base_values(self, node)?;
        let resource_contexts = store.drive_resource_contexts(self);
        let container_unit_mask = store.container_relative_length_unit_mask(self);
        let document_base_url = &self.document_resource_contexts.document_base_url;
        let store = store.view(self);
        use crate::css::computed_value_types::{STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
        use crate::css::style_compute::{
            FfiEffectiveColorSchemeInput, FfiFontMetrics, FfiLengthResolutionContext, FfiStyleComputationEnvironment,
            LONGHAND_DRIVE_PHASE_REMAINING, drive_property_computation, empty_longhand_driver_results,
            is_required_driver_input, parent_snapshot_for_style_record, property_computation_order_for_phase,
        };

        let Some(view) = self.computed_group_sets.base_style_record_view(old_style_record) else {
            counters.bump(Counter::EngineComputedRecordBailRecord);
            return Err(Unanswered::Refused);
        };
        // The font and writing mode for this drive come from the underlying style.
        let payloads = view.payloads;
        // A record holding no table has no slots to copy the unselected properties from; the
        // caller drives it in full.
        let Some(old_table) = (unsafe { view.longhand_table.as_ref() }) else {
            return Ok(PartialDrive::DriverInputMoved);
        };
        let snapshot = match self.record_inheritance_parent(node) {
            None => None,
            Some(parent) => match self.computed_group_sets.assigned_style_record(parent) {
                Some(record) => {
                    let view = self.computed_group_sets.style_record_view(record.raw()).or_refused()?;
                    Some(parent_snapshot_for_style_record(self, record.raw(), unsafe {
                        view.animated_overlay.as_ref()
                    }))
                }
                None => {
                    counters.bump(Counter::EngineComputedRecordBailRecordParent);
                    return Err(Unanswered::Refused);
                }
            },
        };
        let font = unsafe {
            payloads[STYLE_GROUP_INDEX_FONT]
                .cast::<crate::css::computed_value_types::FontValues>()
                .deref()
        };
        let inherited_box = unsafe {
            payloads[STYLE_GROUP_INDEX_INHERITED_BOX]
                .cast::<crate::css::computed_values::InheritedBoxValues>()
                .deref()
        };
        let (width_basis, height_basis) = self.container_unit_bases(
            node,
            container_unit_mask,
            inherited_box.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            inputs,
        );
        let mut resolved_viewport_relative_length = false;
        let length = FfiLengthResolutionContext {
            viewport_width: inputs.viewport_width,
            viewport_height: inputs.viewport_height,
            font_metrics: FfiFontMetrics {
                font_size: font.font_size.to_double(),
                x_height: drive_font_metric(font.font_x_height),
                // The C++ metrics approximate the cap height with the ascent.
                cap_height: drive_font_metric(font.font_ascent),
                zero_advance: drive_font_metric(font.font_zero_advance),
                line_height: font.line_height_used.to_double(),
            },
            root_font_metrics: FfiFontMetrics {
                font_size: inputs.root_font_size,
                x_height: inputs.root_font_x_height,
                cap_height: inputs.root_font_cap_height,
                zero_advance: inputs.root_font_zero_advance,
                line_height: inputs.root_line_height,
            },
            font_metrics_depend_on_viewport_metrics: view.dependency_flags & (1 << 1) != 0,
            root_font_metrics_depend_on_viewport_metrics: inputs.root_font_metrics_depend_on_viewport_metrics,
            has_container_width_basis: width_basis.is_some(),
            has_container_height_basis: height_basis.is_some(),
            container_width_basis: width_basis.map_or(0.0, |basis| basis.basis),
            container_height_basis: height_basis.map_or(0.0, |basis| basis.basis),
            container_width_basis_depends_on_viewport_metrics: width_basis
                .is_some_and(|basis| basis.depends_on_viewport_metrics),
            container_height_basis_depends_on_viewport_metrics: height_basis
                .is_some_and(|basis| basis.depends_on_viewport_metrics),
            subject_inline_axis_is_horizontal: inherited_box.writing_mode
                == crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            resolved_viewport_relative_length: &raw mut resolved_viewport_relative_length,
        };
        // No element fact reaches the remaining phase through this environment: the moved
        // properties were checked not to need one, and the required driver inputs are compared
        // against the record below.
        let tree_counting_inputs = self.element_tree_counting_inputs(node);
        let environment = FfiStyleComputationEnvironment {
            box_type_input: crate::css::style_compute::rust_box_type_transformation_input(
                0,
                crate::css::style_compute::FfiStyleAdjustmentTarget::Element,
                false,
                crate::css::display::FfiDisplay::block(),
            ),
            color_scheme_input: FfiEffectiveColorSchemeInput {
                preferred_color_scheme: 0,
                has_document_supported_schemes: false,
                document_supported_scheme_codes: std::ptr::null(),
                document_supported_scheme_count: 0,
            },
            is_th_element: false,
            has_new_font_size: false,
            has_tree_counting_context: tree_counting_inputs != 0,
            sibling_count: tree_counting_inputs >> 32,
            sibling_index: tree_counting_inputs & u64::from(u32::MAX),
            random_base_values: random_base_values.as_ptr(),
            random_base_value_count: random_base_values.len(),
            document_base_url: document_base_url.as_ptr(),
            document_base_url_length: document_base_url.len(),
            style_sheet_resource_contexts: resource_contexts.as_ptr(),
            style_sheet_resource_context_count: resource_contexts.len(),
            device_pixels_per_css_pixel: inputs.device_pixels_per_css_pixel,
            initial_font_size_raw: inputs.initial_font_size_raw,
            default_font_size_raw: inputs.default_font_size_raw,
        };
        let mut table = view.longhand_table_for_partial_drive();
        let mut results = empty_longhand_driver_results();
        let mut effective_color_scheme = old_table.effective_color_scheme();
        unsafe {
            drive_property_computation(
                &raw mut table,
                std::ptr::null_mut(),
                &store,
                snapshot.as_ref(),
                None,
                &raw const environment,
                u32::MAX,
                selected.as_ptr(),
                LONGHAND_DRIVE_PHASE_REMAINING,
                &raw const length,
                std::ptr::null(),
                std::ptr::null(),
                &raw mut results,
                &mut effective_color_scheme,
                true,
            );
        }
        counters.bump(Counter::EnginePartialDrivesStarted);

        counters.add(
            Counter::EnginePhysicalLonghandEvaluations,
            u64::from(results.longhand_evaluations),
        );
        counters.add(
            Counter::EnginePartialLonghandEvaluations,
            u64::from(results.longhand_evaluations),
        );
        // An `inherit` of a non-inherited property reads the half of the parent's style a child
        // normally cannot see. The value itself is computed here; what C++ does beside it is one
        // write on the parent, which the row leaves for the host to drain after the batch.
        if results.explicitly_inherited_non_inherited_style_groups != 0
            && !self.parent_record_answers_explicit_inheritance(self.tree.flat_tree_parent(node))
        {
            counters.bump(Counter::EngineComputedRecordBailDrive);
            return Err(Unanswered::Refused);
        }
        *explicitly_inherited_groups |= results.explicitly_inherited_non_inherited_style_groups;
        // An input the drive reads for properties it did not select moved with the selection: the
        // caller drives the record in full instead.
        if table.display_before_box_type_transformation() != old_table.display_before_box_type_transformation() {
            return Ok(PartialDrive::DriverInputMoved);
        }
        let old_values = old_table.value_pointers();
        for &property in property_computation_order_for_phase(LONGHAND_DRIVE_PHASE_REMAINING) {
            if !is_required_driver_input(property) {
                continue;
            }
            let slot = usize::from(property - crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID);
            let old_value = old_values[slot];
            let new_value = table
                .get(property)
                .map_or(std::ptr::null(), |value| value.pointer().cast());
            if old_value == new_value {
                continue;
            }
            let equal = unsafe {
                match (
                    old_value.cast::<StyleValueData>().as_ref(),
                    new_value.cast::<StyleValueData>().as_ref(),
                ) {
                    (Some(old_value), Some(new_value)) => old_value == new_value,
                    _ => false,
                }
            };
            if !equal {
                return Ok(PartialDrive::DriverInputMoved);
            }
            table.copy_slot_from(old_table, property);
        }
        // The group builders resolve against the same context; they report no viewport dependence
        // of their own.
        let length = FfiLengthResolutionContext {
            resolved_viewport_relative_length: std::ptr::null_mut(),
            ..length
        };
        Ok(PartialDrive::Driven((
            table,
            length,
            results.longhand_evaluations,
            None,
        )))
    }

    /// Drive a record through every phase: the font phase against the parent's metrics, the
    /// element's font resolved through the document's resolver, line-height and color-scheme
    /// against that font, and the remaining phase with the element facts the box-type
    /// transformation reads. Root-input preparation finishes only the font and line-height
    /// phases and preserves them for completion. Monospace default-size recascade stays in C++.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn engine_full_drive(
        &mut self,
        subject: DriveSubject,
        old_style_record: Option<computed::FinalStyleRecordID>,
        store: &WinnerStore,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        font_scratch: &mut FontDriveScratch,
        goal: FontDriveGoal,
        has_registered_declarations: bool,
        explicitly_inherited_groups: &mut u32,
        counters: &mut Counters,
    ) -> Drive<FullDrive> {
        let random_base_values = store.drive_random_base_values(self, subject.target.node())?;
        let resource_contexts = store.drive_resource_contexts(self);
        let container_unit_mask = store.container_relative_length_unit_mask(self);
        let document_base_url = &self.document_resource_contexts.document_base_url;
        let store = store.view(self);
        use crate::css::computed_value_types::{STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
        use crate::css::css_pixels::CssPixels;
        use crate::css::property_metadata::property_id as prop;
        use crate::css::style_compute::{
            FfiEffectiveColorSchemeInput, FfiFontMetrics, FfiInputLineHeightMetrics, FfiLengthResolutionContext,
            FfiStyleComputationEnvironment, LONGHAND_DRIVE_PHASE_COLOR_SCHEME, LONGHAND_DRIVE_PHASE_FONT,
            LONGHAND_DRIVE_PHASE_LINE_HEIGHT, LONGHAND_DRIVE_PHASE_REMAINING, drive_property_computation,
            effective_display, empty_longhand_driver_results, font_family_is_monospace, keyword,
        };
        use crate::css::table_group_builder::FfiFontGroupBuildInputs;
        use bridge::element_adjustment_fact as fact;

        let DriveSubject {
            target: _,
            parent,
            facts,
            highlight_parent,
        } = subject;
        let has = |bit: u32| facts & bit != 0;
        let is_document_element = has(fact::IS_DOCUMENT_ELEMENT);
        if self.font_resolution.is_none() {
            counters.bump(Counter::EngineComputedRecordBailNoEnvironment);
            return Err(Unanswered::Refused);
        }
        // HACK: A cascade that ends in `font-family: monospace` re-runs the font-size cascade over
        //       the whole ancestor chain against a 13px default instead of the 16px one, which
        //       changes what a keyword size an ancestor declared means.
        let mut recascaded_font_size_reads_viewport = false;
        let recascaded_font_size = if let Some(pending) = font_scratch
            .pending
            .as_ref()
            .filter(|pending| pending.target == subject.target && pending.registered_finalization_ready)
        {
            pending.recascaded_font_size
        } else if store
            .winning_declaration(prop::FONT_FAMILY)
            .is_some_and(|(value, ..)| font_family_is_monospace(unsafe { &*value.cast::<StyleValueData>() }))
        {
            match self.monospace_recascaded_font_size(subject.target, inputs) {
                MonospaceRecascade::Size(recascaded, reads_viewport) => {
                    recascaded_font_size_reads_viewport = reads_viewport;
                    Some(recascaded)
                }
                MonospaceRecascade::AwaitsFont(request) => {
                    font_scratch.request = Some(font_resolution::FontRequest::new(request));
                    return Err(Unanswered::Suspended(Suspension::Font));
                }
            }
        } else {
            None
        };
        let old_table = match old_style_record {
            Some(old_style_record) => {
                let Some(view) = self.computed_group_sets.base_style_record_view(old_style_record) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return Err(Unanswered::Refused);
                };
                // The transitions the old record declares are decided by the step the row owes
                // the host, which runs against the record it moves away from once installed. A
                // record holding no table is driven from a fresh one, like a first record.
                unsafe { view.longhand_table.as_ref() }
            }
            None => None,
        };
        let parent_view = match parent {
            Some(parent) => {
                let sampled_parent = if subject.target.is_pseudo() && parent == subject.target.node() {
                    self.computed_group_sets.sampled_composition_identity_for_pseudo(parent)
                } else {
                    self.computed_group_sets.sampled_composition_identity(parent)
                };
                let Some(parent_record) = sampled_parent
                    .and_then(computed::FinalStyleRecordID::from_raw)
                    .or_else(|| self.computed_group_sets.assigned_style_record(parent))
                else {
                    counters.bump(Counter::EngineComputedRecordBailRecordParent);
                    return Err(Unanswered::Refused);
                };
                let Some(parent_view) = self.computed_group_sets.style_record_view(parent_record.raw()) else {
                    counters.bump(Counter::EngineComputedRecordBailRecordParent);
                    return Err(Unanswered::Refused);
                };
                Some(parent_view)
            }
            None => None,
        };
        // The document element inherits from the initial values and resolves its font against
        // the document's initial font, the way C++'s document resolution context does.
        let initial_metrics = FfiFontMetrics {
            font_size: inputs.initial_font_size,
            x_height: inputs.initial_font_x_height,
            cap_height: inputs.initial_font_cap_height,
            zero_advance: inputs.initial_font_zero_advance,
            line_height: 0.0,
        };
        let (parent_metrics, parent_font_metrics_depend_on_viewport_metrics, parent_line_height_used) =
            match &parent_view {
                Some(parent_view) => {
                    let parent_font = unsafe {
                        parent_view.payloads[STYLE_GROUP_INDEX_FONT]
                            .cast::<crate::css::computed_value_types::FontValues>()
                            .deref()
                    };
                    (
                        FfiFontMetrics {
                            font_size: parent_font.font_size.to_double(),
                            x_height: drive_font_metric(parent_font.font_x_height),
                            cap_height: drive_font_metric(parent_font.font_ascent),
                            zero_advance: drive_font_metric(parent_font.font_zero_advance),
                            line_height: parent_font.line_height_used.to_double(),
                        },
                        parent_view.dependency_flags & (1 << 1) != 0,
                        parent_font.line_height_used.to_double(),
                    )
                }
                None => (initial_metrics, false, 0.0),
            };
        // The parent's display, past any display:contents ancestor, is what the box-type
        // transformation reads.
        let mut parent_display = None;
        let mut ancestor = parent;
        while let Some(current) = ancestor {
            let Some(record) = self.computed_group_sets.assigned_style_record(current) else {
                break;
            };
            let Some(ancestor_view) = self.computed_group_sets.style_record_view(record.raw()) else {
                break;
            };
            let Some(ancestor_table) = (unsafe { ancestor_view.longhand_table.as_ref() }) else {
                break;
            };
            let display = effective_display(ancestor_table, None);
            if !display.is_contents() {
                parent_display = Some(display);
                break;
            }
            ancestor = self.tree.flat_tree_parent(current);
        }
        let snapshot = match &parent_view {
            Some(parent_view) => {
                // Every record the host or the engine assigns to an element holds its table.
                let parent_table = unsafe { parent_view.longhand_table.as_ref() };
                debug_assert!(
                    parent_table.is_some(),
                    "an assigned parent record without a longhand table"
                );
                parent_table.map(|parent_table| {
                    crate::css::style_compute::ParentSnapshot::new(
                        parent_table,
                        unsafe { parent_view.animated_overlay.as_ref() },
                        parent_font_metrics_depend_on_viewport_metrics,
                    )
                })
            }
            None => None,
        };
        let highlight_parent_record = (subject.target.pseudo_kind() == pseudo_kind::SELECTION)
            .then(|| {
                highlight_parent.or_else(|| {
                    self.retained_highlight_inheritance_parent_style_record(
                        subject.target.node(),
                        pseudo_kind::SELECTION,
                    )
                })
            })
            .flatten();
        let highlight_snapshot = highlight_parent_record
            .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
            .and_then(|view| {
                let table = unsafe { view.longhand_table.as_ref() }?;
                Some(crate::css::style_compute::ParentSnapshot::new(
                    table,
                    unsafe { view.animated_overlay.as_ref() },
                    view.dependency_flags & (1 << 1) != 0,
                ))
            });
        let highlight = (subject.target.pseudo_kind() == pseudo_kind::SELECTION)
            .then(|| crate::css::style_compute::HighlightInheritance::new(pseudo_kind::SELECTION, highlight_snapshot));
        // The subject axis is the element's own writing mode when it has one, else its parent's;
        // the initial writing mode is horizontal.
        let inherited_box_payload = match old_style_record {
            Some(old_style_record) => {
                let Some(view) = self.computed_group_sets.style_record_view(old_style_record.raw()) else {
                    counters.bump(Counter::EngineComputedRecordBailRecord);
                    return Err(Unanswered::Refused);
                };
                Some(view.payloads[STYLE_GROUP_INDEX_INHERITED_BOX])
            }
            None => parent_view
                .as_ref()
                .map(|parent_view| parent_view.payloads[STYLE_GROUP_INDEX_INHERITED_BOX]),
        };
        let subject_inline_axis_is_horizontal = inherited_box_payload.is_none_or(|payload| {
            let inherited_box = unsafe {
                payload
                    .cast::<crate::css::computed_values::InheritedBoxValues>()
                    .deref()
            };
            inherited_box.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB
        });
        let (width_basis, height_basis) = self.container_unit_bases(
            subject.target.node(),
            container_unit_mask,
            subject_inline_axis_is_horizontal,
            inputs,
        );
        let document_root_font_metrics = FfiFontMetrics {
            font_size: inputs.root_font_size,
            x_height: inputs.root_font_x_height,
            cap_height: inputs.root_font_cap_height,
            zero_advance: inputs.root_font_zero_advance,
            line_height: inputs.root_line_height,
        };
        let tree_counting_inputs = self.element_tree_counting_inputs(subject.target.node());
        let environment = FfiStyleComputationEnvironment {
            box_type_input: crate::css::style_compute::rust_box_type_transformation_input(
                facts,
                crate::css::style_compute::FfiStyleAdjustmentTarget::Element,
                parent_display.is_some(),
                parent_display.unwrap_or_else(crate::css::display::FfiDisplay::block),
            ),
            color_scheme_input: FfiEffectiveColorSchemeInput {
                preferred_color_scheme: inputs.preferred_color_scheme,
                has_document_supported_schemes: inputs.has_document_supported_schemes,
                document_supported_scheme_codes: inputs.document_supported_scheme_codes.as_ptr(),
                document_supported_scheme_count: usize::from(inputs.document_supported_scheme_count),
            },
            is_th_element: has(fact::IS_TH),
            has_new_font_size: recascaded_font_size.is_some(),
            has_tree_counting_context: tree_counting_inputs != 0,
            sibling_count: tree_counting_inputs >> 32,
            sibling_index: tree_counting_inputs & u64::from(u32::MAX),
            random_base_values: random_base_values.as_ptr(),
            random_base_value_count: random_base_values.len(),
            document_base_url: document_base_url.as_ptr(),
            document_base_url_length: document_base_url.len(),
            style_sheet_resource_contexts: resource_contexts.as_ptr(),
            style_sheet_resource_context_count: resource_contexts.len(),
            device_pixels_per_css_pixel: inputs.device_pixels_per_css_pixel,
            initial_font_size_raw: inputs.initial_font_size_raw,
            default_font_size_raw: inputs.default_font_size_raw,
        };
        let mut resolved_viewport_relative_length = false;
        let resolved_viewport_relative_length_pointer = &raw mut resolved_viewport_relative_length;
        // The root metrics a phase resolves against: the document's, except for the document
        // element itself, whose font phase reads the initial font and whose line-height phase
        // reads its own font; its remaining phase reads the document's metrics as they stand,
        // which C++ refreshes only after computing it.
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
                resolved_viewport_relative_length: resolved_viewport_relative_length_pointer,
            };
        let resumed = font_scratch
            .pending
            .take()
            .filter(|pending| pending.target == subject.target);
        let resuming = resumed.is_some();
        let root_font_complete = resumed.as_ref().is_some_and(|pending| pending.root_font_complete);
        let registered_finalization_ready = resumed
            .as_ref()
            .is_some_and(|pending| pending.registered_finalization_ready);
        if !resuming {
            counters.bump(Counter::EngineFullDrivesStarted);
        }
        let (mut table, mut results, mut effective_color_scheme) = match resumed {
            Some(pending) => {
                resolved_viewport_relative_length = pending.resolved_viewport_relative_length;
                if !pending.root_font_complete {
                    counters.bump(Counter::FontRefillResumedDrives);
                    counters.add(
                        Counter::FontRefillPreservedLonghands,
                        u64::from(pending.results.longhand_evaluations),
                    );
                }
                (pending.table, pending.results, pending.effective_color_scheme)
            }
            None => (
                old_table.map_or_else(ComputedLonghandTable::new, ComputedLonghandTable::copied_for_drive),
                empty_longhand_driver_results(),
                -1,
            ),
        };
        let drive = |counters: &mut Counters,
                     table: &mut ComputedLonghandTable,
                     results: &mut crate::css::style_compute::FfiLonghandDriverResults,
                     effective_color_scheme: &mut i16,
                     phase: u8,
                     length: *const FfiLengthResolutionContext,
                     input_line_height_metrics: *const FfiInputLineHeightMetrics,
                     line_height_before: *const std::ffi::c_void| unsafe {
            let evaluations_before = results.longhand_evaluations;
            drive_property_computation(
                std::ptr::from_mut(table),
                std::ptr::null_mut(),
                &store,
                snapshot.as_ref(),
                highlight.as_ref(),
                &raw const environment,
                u32::MAX,
                std::ptr::null(),
                phase,
                length,
                input_line_height_metrics,
                line_height_before,
                std::ptr::from_mut(results),
                effective_color_scheme,
                true,
            );
            counters.add(
                Counter::EnginePhysicalLonghandEvaluations,
                u64::from(results.longhand_evaluations - evaluations_before),
            );
        };
        let font_length = if is_document_element {
            length_context(initial_metrics, false, initial_metrics, false)
        } else {
            length_context(
                parent_metrics,
                parent_font_metrics_depend_on_viewport_metrics,
                document_root_font_metrics,
                inputs.root_font_metrics_depend_on_viewport_metrics,
            )
        };
        if !resuming {
            // The recascaded size stands in for the element's cascaded `font-size`, which the drive
            // then leaves alone, exactly as C++ writes it into the working set before driving. An
            // element that declares its own `font-size` still has that declaration win, because the
            // drive reads a winning declaration before it consults this.
            if let Some(recascaded) = recascaded_font_size {
                table.set_computed(
                    prop::FONT_SIZE,
                    StyleValueData::Length {
                        value: CssPixels::from_raw(recascaded).to_double(),
                        unit: crate::css::style_compute::px_length_unit(),
                    },
                    -1,
                );
            }
            drive(
                counters,
                &mut table,
                &mut results,
                &mut effective_color_scheme,
                LONGHAND_DRIVE_PHASE_FONT,
                &raw const font_length,
                std::ptr::null(),
                std::ptr::null(),
            );
            // A recascaded size that read the viewport makes the element's style and font
            // metrics read it, as C++ marks them beside the size it writes.
            if recascaded_font_size_reads_viewport {
                results.depends_on_viewport_metrics = true;
                results.font_metrics_depend_on_viewport_metrics = true;
            }
        }

        // The element's own font, resolved as the C++ font computer would for these values.
        let value_of = |table: &ComputedLonghandTable, property: u16| -> Option<&StyleValueData> {
            unsafe {
                table
                    .effective_value(None, property, true)
                    .value
                    .cast::<StyleValueData>()
                    .as_ref()
            }
        };
        // The font size the element's own lengths resolve against is the C++ working set's, a
        // CSSPixels value, not the computed value's double. The font phase computes font-size,
        // font-weight, and font-width to these types: a value it cannot compute is `unset`.
        let font_size = match value_of(&table, prop::FONT_SIZE) {
            Some(StyleValueData::Length { value, unit }) if *unit == crate::css::style_compute::px_length_unit() => {
                CssPixels::nearest_value_for(*value).to_double()
            }
            _ => {
                debug_assert!(false, "the font phase left font-size uncomputed");
                CssPixels::from_raw(inputs.initial_font_size_raw).to_double()
            }
        };
        let font_size_raw = CssPixels::nearest_value_for(font_size).raw_value();
        let font_family = table.effective_value(None, prop::FONT_FAMILY, true).value;
        let font_slope = match value_of(&table, prop::FONT_STYLE) {
            Some(StyleValueData::FontStyle { font_style, .. }) => match *font_style {
                crate::css::css_enums::font_style_keyword::ITALIC => 1,
                crate::css::css_enums::font_style_keyword::OBLIQUE => 2,
                _ => 0,
            },
            _ => 0,
        };
        let (font_weight, font_width) = match (value_of(&table, prop::FONT_WEIGHT), value_of(&table, prop::FONT_WIDTH))
        {
            (Some(StyleValueData::Number { value: weight }), Some(StyleValueData::Percentage { value: width })) => {
                (*weight, *width)
            }
            _ => {
                debug_assert!(false, "the font phase left font-weight or font-width uncomputed");
                (400.0, 100.0)
            }
        };
        let font_optical_sizing = match value_of(&table, prop::FONT_OPTICAL_SIZING) {
            Some(StyleValueData::Keyword { keyword }) => {
                crate::css::css_enums::keyword_to_font_optical_sizing(*keyword).unwrap_or(0)
            }
            _ => 0,
        };
        // The resolver reads these beside the family, so the request names each one whose value
        // is not the initial one and nothing for the rest. Read the computed values: a
        // non-default setting can also come from inheritance.
        let font_feature_values: [bridge::FfiHostHandle; bridge::FONT_RESOLUTION_FEATURE_INPUT_COUNT] = {
            let inputs = [
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
                let (property, default_keyword) = inputs[index];
                let pointer = match value_of(&table, property) {
                    Some(StyleValueData::Keyword { keyword }) if *keyword == default_keyword => std::ptr::null(),
                    _ => table.effective_value(None, property, true).value,
                };
                bridge::FfiHostHandle::from_pointer(pointer.cast())
            })
        };
        let request = bridge::FfiFontResolutionRequest {
            font_family: bridge::FfiHostHandle::from_pointer(font_family.cast()),
            tree_scope: self.tree.tree_scope(subject.target.node()).0,
            font_feature_values,
            font_size_raw,
            font_slope,
            font_weight,
            font_width,
            font_optical_sizing,
            font_environment_generation: inputs.font_environment_generation,
        };
        let Some(resolved) = self
            .font_resolution
            .as_ref()
            .and_then(|resolutions| resolutions.lookup(request))
        else {
            font_scratch.request = Some(font_resolution::FontRequest::new(request));
            font_scratch.pending = Some(PendingFontDrive {
                target: subject.target,
                root_font_complete: false,
                registered_finalization_ready: false,
                recascaded_font_size,
                table,
                results,
                effective_color_scheme,
                resolved_viewport_relative_length,
            });
            return Err(Unanswered::Suspended(Suspension::Font));
        };
        let own_metrics = |line_height: f64| FfiFontMetrics {
            font_size,
            x_height: drive_font_metric(resolved.x_height),
            cap_height: drive_font_metric(resolved.ascent),
            zero_advance: drive_font_metric(resolved.zero_advance),
            line_height,
        };

        let line_height_length = if is_document_element {
            length_context(
                own_metrics(parent_line_height_used),
                results.font_metrics_depend_on_viewport_metrics,
                own_metrics(parent_line_height_used),
                results.font_metrics_depend_on_viewport_metrics,
            )
        } else {
            length_context(
                own_metrics(parent_line_height_used),
                results.font_metrics_depend_on_viewport_metrics,
                document_root_font_metrics,
                inputs.root_font_metrics_depend_on_viewport_metrics,
            )
        };
        if !root_font_complete {
            drive(
                counters,
                &mut table,
                &mut results,
                &mut effective_color_scheme,
                LONGHAND_DRIVE_PHASE_LINE_HEIGHT,
                &raw const line_height_length,
                std::ptr::null(),
                std::ptr::null(),
            );
        }

        // The used line height, as the C++ working set reads it from the computed value.
        let normal_line_height = f64::from(resolved.ascent.round() as i32 + resolved.descent.round() as i32);
        // The line-height phase computes line-height to one of these; a value it cannot compute
        // is `unset`, so any other shape is read as `normal`.
        let line_height_used = |table: &ComputedLonghandTable| -> f64 {
            match value_of(table, prop::LINE_HEIGHT) {
                Some(StyleValueData::Keyword { keyword }) if *keyword == keyword::NORMAL => normal_line_height,
                Some(StyleValueData::Length { value, unit })
                    if *unit == crate::css::style_compute::px_length_unit() =>
                {
                    CssPixels::nearest_value_for(*value).to_double()
                }
                Some(StyleValueData::Number { value }) => CssPixels::nearest_value_for(value * font_size).to_double(),
                _ => {
                    debug_assert!(false, "the line-height phase left line-height uncomputed");
                    normal_line_height
                }
            }
        };
        let line_height_before_adjustments = line_height_used(&table);
        if goal == FontDriveGoal::RootInputs {
            let root_inputs = RootFontInputs {
                metrics: [
                    font_size.to_bits(),
                    drive_font_metric(resolved.x_height).to_bits(),
                    drive_font_metric(resolved.ascent).to_bits(),
                    drive_font_metric(resolved.zero_advance).to_bits(),
                    line_height_before_adjustments.to_bits(),
                ],
                depends_on_viewport: results.font_metrics_depend_on_viewport_metrics,
            };
            font_scratch.pending = Some(PendingFontDrive {
                target: subject.target,
                root_font_complete: true,
                registered_finalization_ready: false,
                recascaded_font_size,
                table,
                results,
                effective_color_scheme,
                resolved_viewport_relative_length,
            });
            return Ok(FullDrive::RootInputs(root_inputs));
        }
        if !registered_finalization_ready {
            drive(
                counters,
                &mut table,
                &mut results,
                &mut effective_color_scheme,
                LONGHAND_DRIVE_PHASE_COLOR_SCHEME,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            );
        }
        effective_color_scheme = table.effective_color_scheme();

        // `rem` on the document element names the document element's own computed font-size, which
        // this drive has just resolved, the way the line-height phase above already reads it. The
        // document's retained root metrics still describe the font the root had before.
        let remaining_length = if is_document_element {
            length_context(
                own_metrics(line_height_before_adjustments),
                results.font_metrics_depend_on_viewport_metrics,
                own_metrics(line_height_before_adjustments),
                results.font_metrics_depend_on_viewport_metrics,
            )
        } else {
            length_context(
                own_metrics(line_height_before_adjustments),
                results.font_metrics_depend_on_viewport_metrics,
                document_root_font_metrics,
                inputs.root_font_metrics_depend_on_viewport_metrics,
            )
        };
        if has_registered_declarations && !registered_finalization_ready {
            let registered_context = custom_property_cascade::RegisteredValueContext {
                length: FfiLengthResolutionContext {
                    resolved_viewport_relative_length: std::ptr::null_mut(),
                    ..remaining_length
                },
                color_scheme: effective_color_scheme as u8,
            };
            font_scratch.pending = Some(PendingFontDrive {
                target: subject.target,
                root_font_complete: true,
                registered_finalization_ready: true,
                recascaded_font_size,
                table,
                results,
                effective_color_scheme,
                resolved_viewport_relative_length,
            });
            return Ok(FullDrive::AwaitsRegisteredContext(registered_context));
        }
        let input_line_height_metrics = if has(fact::CHECK_INPUT_LINE_HEIGHT) {
            FfiInputLineHeightMetrics {
                current_line_height: line_height_before_adjustments,
                minimum_line_height: normal_line_height,
            }
        } else {
            FfiInputLineHeightMetrics {
                current_line_height: 0.0,
                minimum_line_height: 0.0,
            }
        };
        let line_height_value = table.effective_value(None, prop::LINE_HEIGHT, true).value;
        drive(
            counters,
            &mut table,
            &mut results,
            &mut effective_color_scheme,
            LONGHAND_DRIVE_PHASE_REMAINING,
            &raw const remaining_length,
            &raw const input_line_height_metrics,
            line_height_value,
        );
        // An `inherit` of a non-inherited property reads the half of the parent's style a child
        // normally cannot see. The value itself is computed here; what C++ does beside it is one
        // write on the parent, which the row leaves for the host to drain after the batch.
        if results.explicitly_inherited_non_inherited_style_groups != 0
            && !self.parent_record_answers_explicit_inheritance(subject.parent)
        {
            counters.bump(Counter::EngineComputedRecordBailDrive);
            return Err(Unanswered::Refused);
        }
        *explicitly_inherited_groups |= results.explicitly_inherited_non_inherited_style_groups;
        let line_height_used_after = line_height_used(&table);
        let keyword_code = |property: u16, map: fn(u16) -> Option<u8>| match value_of(&table, property) {
            Some(StyleValueData::Keyword { keyword }) => map(*keyword).unwrap_or(0),
            _ => 0,
        };
        let math_depth = match value_of(&table, prop::MATH_DEPTH) {
            Some(StyleValueData::Integer { value }) => *value,
            _ => 0,
        };
        let font = FfiFontGroupBuildInputs {
            font_size_raw,
            line_height_used_raw: CssPixels::nearest_value_for(line_height_used_after).raw_value(),
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
            font_weight,
            font_width,
            math_shift: keyword_code(prop::MATH_SHIFT, crate::css::css_enums::keyword_to_math_shift),
            math_style: keyword_code(prop::MATH_STYLE, crate::css::css_enums::keyword_to_math_style),
            math_depth,
        };
        let length = FfiLengthResolutionContext {
            resolved_viewport_relative_length: std::ptr::null_mut(),
            ..remaining_length
        };
        Ok(FullDrive::Driven((
            table,
            length,
            results.longhand_evaluations,
            Some(font),
        )))
    }
}

/// Whether a computed table's `animation-name` names any animation.
pub(super) fn table_names_animations(table: &ComputedLonghandTable) -> bool {
    use crate::css::style_compute::keyword;
    let is_none =
        |value: &StyleValueData| matches!(value, StyleValueData::Keyword { keyword: name } if *name == keyword::NONE);
    table
        .get(crate::css::property_metadata::property_id::ANIMATION_NAME)
        .is_some_and(|value| match value.data() {
            StyleValueData::ValueList { values, .. } => values.as_slice().iter().any(|value| !is_none(value.data())),
            value => !is_none(value),
        })
}

/// A font's pixel metric as the drive resolves font-relative units against it: the C++ length
/// resolution context carries the metrics as `CSSPixels`, so an `ex` resolves against the
/// fixed-point x-height rather than the font's raw floating-point one.
pub(crate) fn drive_font_metric(value: f32) -> f64 {
    crate::css::css_pixels::CssPixels::nearest_value_for_f32(value).to_double()
}
