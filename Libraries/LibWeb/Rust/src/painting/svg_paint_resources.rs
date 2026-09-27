/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::css::computed_value_types::ComputedResolvedTransform;
use crate::layout::node_data::NodeSlotId;
use crate::painting::host::{FfiSvgGradientDescription, FfiSvgPatternDescription};
use crate::painting::svg_filter::SvgFilterPrimitive;
use libgfx_rust::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SvgPaintResourceKind {
    Filter,
    BackdropFilter,
    Fill,
    Stroke,
}

impl SvgPaintResourceKind {
    pub(crate) const ALL: [Self; 4] = [Self::Filter, Self::BackdropFilter, Self::Fill, Self::Stroke];

    pub(crate) const fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PublishedSvgGradientStop {
    pub color: Color,
    pub position: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PublishedSvgGradient {
    pub description: FfiSvgGradientDescription,
    pub stops: Vec<PublishedSvgGradientStop>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PublishedSvgPattern {
    pub description: FfiSvgPatternDescription,
    pub css_transform: Vec<ComputedResolvedTransform>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum PublishedSvgPaintServer {
    #[default]
    None,
    Gradient(PublishedSvgGradient),
    Pattern(PublishedSvgPattern),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PublishedSvgFilter {
    pub failed: bool,
    pub primitives: Vec<SvgFilterPrimitive>,
}

#[derive(Clone, Default)]
pub(crate) struct SvgPaintResourceRow {
    enrolled_kinds: u8,
    filter: Option<Arc<PublishedSvgFilter>>,
    backdrop_filter: Option<Arc<PublishedSvgFilter>>,
    fill: Option<Arc<PublishedSvgPaintServer>>,
    stroke: Option<Arc<PublishedSvgPaintServer>>,
}

impl SvgPaintResourceRow {
    fn published_filter(&self, kind: SvgPaintResourceKind) -> Option<Arc<PublishedSvgFilter>> {
        match kind {
            SvgPaintResourceKind::Filter => self.filter.clone(),
            SvgPaintResourceKind::BackdropFilter => self.backdrop_filter.clone(),
            SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke => unreachable!(),
        }
    }

    fn published_paint_server(&self, kind: SvgPaintResourceKind) -> Option<Arc<PublishedSvgPaintServer>> {
        match kind {
            SvgPaintResourceKind::Fill => self.fill.clone(),
            SvgPaintResourceKind::Stroke => self.stroke.clone(),
            SvgPaintResourceKind::Filter | SvgPaintResourceKind::BackdropFilter => unreachable!(),
        }
    }

    fn filter_entry(&mut self, kind: SvgPaintResourceKind) -> &mut Option<Arc<PublishedSvgFilter>> {
        match kind {
            SvgPaintResourceKind::Filter => &mut self.filter,
            SvgPaintResourceKind::BackdropFilter => &mut self.backdrop_filter,
            SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke => unreachable!(),
        }
    }

    fn paint_server_entry(&mut self, kind: SvgPaintResourceKind) -> &mut Option<Arc<PublishedSvgPaintServer>> {
        match kind {
            SvgPaintResourceKind::Fill => &mut self.fill,
            SvgPaintResourceKind::Stroke => &mut self.stroke,
            SvgPaintResourceKind::Filter | SvgPaintResourceKind::BackdropFilter => unreachable!(),
        }
    }

    fn forget_published(&mut self, kinds: u8) {
        for kind in SvgPaintResourceKind::ALL {
            if kinds & kind.bit() == 0 {
                continue;
            }
            match kind {
                SvgPaintResourceKind::Filter | SvgPaintResourceKind::BackdropFilter => *self.filter_entry(kind) = None,
                SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke => *self.paint_server_entry(kind) = None,
            }
        }
    }
}

fn publish<T: PartialEq>(entry: &mut Option<Arc<T>>, value: T) -> bool {
    if entry.as_ref().is_some_and(|previous| **previous == value) {
        return false;
    }
    *entry = Some(Arc::new(value));
    true
}

/// Each enrolled row's published SVG paint resources. A frame shares the table, so a write copies
/// it only while a published frame still holds it.
pub(crate) type SvgPaintResourceRows = HashMap<NodeSlotId, SvgPaintResourceRow>;

#[derive(Default)]
pub(crate) struct SvgPaintResources {
    rows: RefCell<Arc<SvgPaintResourceRows>>,
    needs_sync: Cell<bool>,
}

/// The published filter of a kind in a slot's row of `rows`.
pub(crate) fn published_filter_in(
    rows: &SvgPaintResourceRows,
    slot: NodeSlotId,
    kind: SvgPaintResourceKind,
) -> Option<Arc<PublishedSvgFilter>> {
    rows.get(&slot)?.published_filter(kind)
}

/// The image frames of the published SVG filters in `rows`, once each.
pub(crate) fn published_filter_image_frames_in(
    rows: &SvgPaintResourceRows,
) -> Vec<libgfx_rust::image_frame::ImageFrameHandle> {
    let mut frames: Vec<libgfx_rust::image_frame::ImageFrameHandle> = Vec::new();
    let mut known_ids = std::collections::HashSet::new();
    for row in rows.values() {
        for filter in [&row.filter, &row.backdrop_filter].into_iter().flatten() {
            for frame in filter
                .primitives
                .iter()
                .filter_map(|primitive| primitive.image_frame.as_ref())
            {
                if known_ids.insert(frame.id()) {
                    frames.push(frame.clone());
                }
            }
        }
    }
    frames
}

/// The published paint server of a kind in a slot's row of `rows`.
pub(crate) fn published_paint_server_in(
    rows: &SvgPaintResourceRows,
    slot: NodeSlotId,
    kind: SvgPaintResourceKind,
) -> Option<Arc<PublishedSvgPaintServer>> {
    rows.get(&slot)?.published_paint_server(kind)
}

impl SvgPaintResources {
    fn rows_mut(&self) -> std::cell::RefMut<'_, SvgPaintResourceRows> {
        std::cell::RefMut::map(self.rows.borrow_mut(), Arc::make_mut)
    }

    /// The table as it is now, for a frame to publish.
    pub(crate) fn publish(&self) -> Arc<SvgPaintResourceRows> {
        self.rows.borrow().clone()
    }

    pub(crate) fn set_enrolled_kinds(&self, slot: NodeSlotId, kinds: u8) {
        if kinds == 0 {
            self.forget_slot(slot);
            return;
        }
        self.needs_sync.set(true);
        if self
            .rows
            .borrow()
            .get(&slot)
            .is_some_and(|row| row.enrolled_kinds == kinds)
        {
            return;
        }
        let mut rows = self.rows_mut();
        let row = rows.entry(slot).or_default();
        let previous_kinds = std::mem::replace(&mut row.enrolled_kinds, kinds);
        row.forget_published(previous_kinds & !kinds);
    }

    pub(crate) fn withdraw(&self, slot: NodeSlotId, kind: SvgPaintResourceKind) {
        if !self.rows.borrow().contains_key(&slot) {
            return;
        }
        let mut rows = self.rows_mut();
        let Some(row) = rows.get_mut(&slot) else {
            return;
        };
        row.enrolled_kinds &= !kind.bit();
        row.forget_published(kind.bit());
        if row.enrolled_kinds == 0 {
            rows.remove(&slot);
        }
    }

    pub(crate) fn forget_slot(&self, slot: NodeSlotId) {
        if self.rows.borrow().contains_key(&slot) {
            self.rows_mut().remove(&slot);
        }
    }

    pub(crate) fn has_enrolled_entries(&self) -> bool {
        !self.rows.borrow().is_empty()
    }

    pub(crate) fn note_changed(&self) -> bool {
        if !self.has_enrolled_entries() {
            return false;
        }
        self.needs_sync.set(true);
        true
    }

    pub(crate) fn needs_sync(&self) -> bool {
        self.needs_sync.get()
    }

    pub(crate) fn take_needs_sync(&self) -> bool {
        self.needs_sync.replace(false)
    }

    pub(crate) fn enrolled_entries(&self) -> Vec<(NodeSlotId, SvgPaintResourceKind)> {
        self.rows
            .borrow()
            .iter()
            .flat_map(|(slot, row)| {
                SvgPaintResourceKind::ALL
                    .into_iter()
                    .filter(move |kind| row.enrolled_kinds & kind.bit() != 0)
                    .map(move |kind| (*slot, kind))
            })
            .collect()
    }

    pub(crate) fn published_filter(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
    ) -> Option<Arc<PublishedSvgFilter>> {
        published_filter_in(&self.rows.borrow(), slot, kind)
    }

    pub(crate) fn published_filter_image_frames(&self) -> Vec<libgfx_rust::image_frame::ImageFrameHandle> {
        published_filter_image_frames_in(&self.rows.borrow())
    }

    pub(crate) fn published_paint_server(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
    ) -> Option<Arc<PublishedSvgPaintServer>> {
        published_paint_server_in(&self.rows.borrow(), slot, kind)
    }

    pub(crate) fn publish_paint_server(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
        paint_server: PublishedSvgPaintServer,
    ) -> bool {
        match self.rows.borrow().get(&slot) {
            None => return false,
            Some(row)
                if row
                    .published_paint_server(kind)
                    .is_some_and(|previous| *previous == paint_server) =>
            {
                return false;
            }
            Some(_) => {}
        }
        publish(
            self.rows_mut().get_mut(&slot).unwrap().paint_server_entry(kind),
            paint_server,
        )
    }

    pub(crate) fn publish_filter(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
        filter: PublishedSvgFilter,
    ) -> bool {
        match self.rows.borrow().get(&slot) {
            None => return false,
            Some(row) if row.published_filter(kind).is_some_and(|previous| *previous == filter) => return false,
            Some(_) => {}
        }
        publish(self.rows_mut().get_mut(&slot).unwrap().filter_entry(kind), filter)
    }
}
