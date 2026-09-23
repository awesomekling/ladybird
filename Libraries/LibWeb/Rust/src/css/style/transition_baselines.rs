/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The before-change styles a style stabilization epoch decides CSS transitions against.

use super::RetainedState;
use super::tree::StyleNodeID;
use crate::css::animated_overlay::FfiAnimatedOverlayEntry;
use crate::css::computed_longhand_table::ComputedLonghandTable;
use crate::css::style_value::StyleValueData;

/// An animated value a record inherited, as the nearest ancestor that animates the property holds
/// it: the overlay entry and the base value beneath it.
#[derive(Clone, Copy)]
pub(crate) struct InheritedAnimatedValue<'a> {
    pub(crate) entry: &'a FfiAnimatedOverlayEntry,
    pub(crate) base_value: *const StyleValueData,
}

impl RetainedState {
    /// https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
    /// Style, layout or animation feedback can give a target a transition in any later pass of
    /// the epoch, and that transition starts from the style the target held before the epoch's
    /// first pass. The first record named for a target is that style: it is kept, pinned, until
    /// the epoch commits. Says whether this call recorded it.
    pub(crate) fn record_transition_baseline(&mut self, node: StyleNodeID, pseudo_kind: u8, style_record: u64) -> bool {
        if style_record == 0 || self.transition_baselines.contains_key(&(node, pseudo_kind)) {
            return false;
        }
        self.computed_group_sets.pin_style_record(style_record);
        self.transition_baselines.insert((node, pseudo_kind), style_record);
        true
    }

    /// The before-change style the epoch decides the target's transitions against, or 0 before a
    /// pass has recorded one.
    pub(crate) fn transition_baseline(&self, node: StyleNodeID, pseudo_kind: u8) -> u64 {
        self.transition_baselines
            .get(&(node, pseudo_kind))
            .copied()
            .unwrap_or(0)
    }

    /// The epoch committed: no later pass decides against these styles.
    pub(crate) fn release_transition_baselines(&mut self) {
        for (_, style_record) in std::mem::take(&mut self.transition_baselines) {
            self.computed_group_sets.unpin_style_record(style_record);
        }
    }

    /// A retired node's identity can name another element before the epoch commits.
    pub(crate) fn release_transition_baselines_of(&mut self, node: StyleNodeID) {
        let computed_group_sets = &mut self.computed_group_sets;
        self.transition_baselines.retain(|(target, _), style_record| {
            if *target != node {
                return true;
            }
            computed_group_sets.unpin_style_record(*style_record);
            false
        });
    }

    /// Where an inherited value of `property` in `table`, a record for `node`, comes from when an
    /// ancestor animates it: the nearest ancestor along the chain of records that inherited the
    /// property and holds an overlay entry for it, and the base value of the ancestor the chain
    /// starts at. None when no such ancestor exists.
    pub(crate) fn inherited_animated_value(
        &self,
        node: StyleNodeID,
        table: &ComputedLonghandTable,
        property: u16,
    ) -> Option<InheritedAnimatedValue<'_>> {
        let record_parts = |node: StyleNodeID| {
            let record = self.computed_group_sets.assigned_style_record(node)?;
            let view = self.computed_group_sets.style_record_view(record.raw())?;
            Some((unsafe { view.longhand_table.as_ref() }?, unsafe {
                view.animated_overlay.as_ref()
            }))
        };
        let mut entry = None;
        let mut inherits = table.is_inherited(property);
        let mut ancestor = self.tree.inheritance_parent(node);
        while inherits {
            let current = ancestor?;
            let (ancestor_table, overlay) = record_parts(current)?;
            // NB: A record the engine derived holds an inherited animated value in its table, so
            //     the base value is read where the chain of inheriting records starts.
            entry = entry.or_else(|| overlay.and_then(|overlay| overlay.get(property)));
            inherits = ancestor_table.is_inherited(property);
            if !inherits {
                let base_value = crate::css::style_compute::ParentSnapshot::new(ancestor_table, None, false, false)
                    .value(property)
                    .map_or(std::ptr::null(), std::ptr::from_ref);
                return entry
                    .filter(|_| !base_value.is_null())
                    .map(|entry| InheritedAnimatedValue { entry, base_value });
            }
            ancestor = self.tree.inheritance_parent(current);
        }
        None
    }
}
