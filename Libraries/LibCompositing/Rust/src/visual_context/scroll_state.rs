/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css_pixels::CssPixelPoint;
use crate::display_list::commands::{SpatialNodeIndex, VISUAL_VIEWPORT_NODE_INDEX};
use crate::fast_hash::FastMap;
use crate::node_slot_id::NodeSlotId;
use libgfx_rust::FloatPoint;
use std::cell::RefCell;

// Where a box's scroll offset is, as the render side sees it.
//
// The offset itself is still stored on the DOM - on the element, on the synthetic pseudo-element,
// or on the navigable for the viewport - and this column is the copy the render side can read
// without asking for it. It is kept in step at the three moments the stored offset can change:
// a box becoming the box of something that holds an offset, a write of that stored offset, and a
// move of the viewport's offset. A box whose offset is zero has no entry, which is the common
// case by a long way.
//
// Keyed by the paintable row rather than by the DOM node, because that is the key the render side
// has. A row's id carries the generation of the slot it came from, so an entry left behind by a
// freed row names nothing a live row can ask for; a row built in a recycled slot publishes its own
// offset as it is built.
#[derive(Default)]
pub struct ScrollOffsetColumn {
    offsets: RefCell<FastMap<NodeSlotId, CssPixelPoint>>,
}

impl ScrollOffsetColumn {
    pub fn offset(&self, slot: NodeSlotId) -> CssPixelPoint {
        self.offsets.borrow().get(&slot).copied().unwrap_or_default()
    }

    pub fn publish(&self, slot: NodeSlotId, offset: CssPixelPoint) {
        if slot.is_invalid() {
            return;
        }
        let mut offsets = self.offsets.borrow_mut();
        if offset == CssPixelPoint::default() {
            offsets.remove(&slot);
        } else {
            offsets.insert(slot, offset);
        }
    }

    pub fn forget(&self, slot: NodeSlotId) {
        self.offsets.borrow_mut().remove(&slot);
    }
}

pub type ScrollStateSlot = usize;
// Position of a scroll or sticky node's entry in the ScrollState store. Slot 0 is the viewport's
// scroll node, so "no slot" needs a value outside the vector.
pub const NO_SCROLL_STATE_SLOT: ScrollStateSlot = usize::MAX;

// Registry entry of one scroll-like node of the accumulated visual context tree, addressed by the
// slot stamped into the node's payload. A scroll container's entry carries its offset; a sticky
// node's entry only links it into the scroll-parent chain, which the anchor default-scroll-shift
// derivation walks and which decides the node's scroller and parent sticky. That chain follows
// containing blocks, deliberately unlike the visual context parent chain for sticky content inside
// fixed-position ancestors.
#[derive(Clone, Debug)]
pub struct ScrollNodeState {
    pub paintable: NodeSlotId,
    pub is_sticky: bool,
    pub node_index: SpatialNodeIndex,
    pub parent_slot: ScrollStateSlot,
    pub own_offset: CssPixelPoint,
}

// Registry of the scroll-like nodes of the accumulated visual context tree: the tree owns
// structure and identity, entries here carry the scroll containers' offsets and the
// containing-block-derived scroll-parent references. Rebuilt together with the tree; offsets are
// refreshed in place between rebuilds. Sticky offsets never live here: they are derived from the
// tree when the snapshot is resolved.
#[derive(Default)]
pub struct ScrollState {
    pub states: Vec<ScrollNodeState>,
    // Whether any box but the viewport could take a wheel event. A registry reset keeps it, so
    // the rebuild that follows sees a change only when the document's scrollers changed.
    pub has_non_viewport_wheel_scroll_target_candidate: bool,
}

impl ScrollState {
    pub fn clear(&mut self) {
        self.states.clear();
    }

    pub fn slot_count(&self) -> usize {
        self.states.len()
    }

    fn append(&mut self, state: ScrollNodeState) -> ScrollStateSlot {
        self.states.push(state);
        self.states.len() - 1
    }

    pub fn register_scroll_node(
        &mut self,
        node_index: SpatialNodeIndex,
        paintable: NodeSlotId,
        parent_slot: ScrollStateSlot,
    ) -> ScrollStateSlot {
        self.append(ScrollNodeState {
            paintable,
            is_sticky: false,
            node_index,
            parent_slot,
            own_offset: CssPixelPoint::default(),
        })
    }

    pub fn register_sticky_node(
        &mut self,
        node_index: SpatialNodeIndex,
        paintable: NodeSlotId,
        parent_slot: ScrollStateSlot,
    ) -> ScrollStateSlot {
        self.append(ScrollNodeState {
            paintable,
            is_sticky: true,
            node_index,
            parent_slot,
            own_offset: CssPixelPoint::default(),
        })
    }

    pub fn state_at_slot(&self, slot: ScrollStateSlot) -> &ScrollNodeState {
        &self.states[slot]
    }

    pub fn state_at_slot_mut(&mut self, slot: ScrollStateSlot) -> &mut ScrollNodeState {
        &mut self.states[slot]
    }

    pub fn node_index_for_slot(&self, slot: ScrollStateSlot) -> SpatialNodeIndex {
        if slot == NO_SCROLL_STATE_SLOT {
            return VISUAL_VIEWPORT_NODE_INDEX;
        }
        self.states[slot].node_index
    }

    pub fn nearest_scrolling_ancestor_slot(&self, slot: ScrollStateSlot) -> ScrollStateSlot {
        let mut ancestor = self.states[slot].parent_slot;
        while ancestor != NO_SCROLL_STATE_SLOT {
            let state = &self.states[ancestor];
            if !state.is_sticky {
                return ancestor;
            }
            ancestor = state.parent_slot;
        }
        NO_SCROLL_STATE_SLOT
    }

    pub fn snapshot(&self, device_pixels_per_css_pixel: f64) -> Vec<FloatPoint> {
        let scale = device_pixels_per_css_pixel as f32;
        let mut snapshot: Vec<FloatPoint> = Vec::new();
        for state in &self.states {
            if state.is_sticky {
                continue;
            }
            let node_index = state.node_index.0 as usize;
            if snapshot.len() <= node_index {
                snapshot.resize(node_index + 1, FloatPoint::default());
            }
            snapshot[node_index] = FloatPoint {
                x: state.own_offset.x.to_float() * scale,
                y: state.own_offset.y.to_float() * scale,
            };
        }
        snapshot
    }
}
