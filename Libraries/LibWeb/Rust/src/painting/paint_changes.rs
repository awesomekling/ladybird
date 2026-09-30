/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The main thread's writes to a document's paint state, which the render owner applies in the order the main thread
//! made them, before the next unit or question that reaches the arena.

use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::painting::ffi::ScrollDirection;
use crate::painting::paintable_data::PaintableFlag;
use crate::painting::record::damage::PaintDamage;
use crate::painting::visual_context::dirty::{VisualContextBoxDirtyKind, VisualContextGlobalRebuildReason};
use std::ffi::c_void;

/// A write the main thread makes to a document's paint state.
pub(crate) enum PaintChange {
    /// A scrollbar of a box grew or shrank back as the pointer moved over it or away.
    ScrollbarEnlarged {
        slot: NodeSlotId,
        direction: ScrollDirection,
        enlarged: bool,
    },
    /// The nearest self-painting inline box around a box paints again.
    NearestSelfPaintingInlineRepaint(NodeSlotId),
    /// The document's selection, which the rows it covers paint.
    Selection(crate::painting::selection::SelectionSnapshot),
    /// The document's selection is gone from the rows below `viewport`.
    SelectionCleared { viewport: NodeSlotId },
    /// The count of scrollable overflow recalculations starts over, for tests.
    ScrollableOverflowRecalculationCountReset,
    /// What of a box changed that its visual context nodes are built from.
    VisualContextBoxDirty {
        slot: NodeSlotId,
        kind: VisualContextBoxDirtyKind,
    },
    /// The visual context tree is built anew at its next update.
    VisualContextFullRebuild(VisualContextGlobalRebuildReason),
    /// Scroll offsets changed: the scroll state is refreshed before it is read next.
    ScrollStateInvalidated,
    /// What a replaced box's DOM node paints from. Where they changed, the box's rows take `damage_when_changed` too.
    ReplacedPaintFacts {
        slot: NodeSlotId,
        facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts,
        damage_when_changed: PaintDamage,
    },
    /// What a box's layer images paint from.
    LayerImagePaintFacts {
        slot: NodeSlotId,
        entries: Vec<crate::painting::layer_image_paint_facts::LayerImagePaintFactsEntry>,
    },
    /// The `<area>` elements of the image map an image box's image is associated with.
    ImageMapAreas {
        slot: NodeSlotId,
        areas: Box<[crate::painting::image_map_areas::PublishedImageMapArea]>,
    },
    /// The scroll offset a box's DOM node stores, and whether it stores one at all.
    ScrollOffset {
        slot: NodeSlotId,
        offset: crate::css::css_pixels::CssPixelPoint,
        dom_target_stores_offset: bool,
    },
    /// What the render side reads of the viewport it draws into.
    VisualContextTreeInputs(crate::painting::host::FfiVisualContextTreeInputs),
    /// A box paints again: its own paint cache, or with `propagated_text_decorations` the text decorations its
    /// descendants inherit from it.
    PaintCacheInvalidated {
        slot: NodeSlotId,
        propagated_text_decorations: bool,
    },
    /// A box paints again, as a repaint of it does, with or without its hit-test items.
    Repaint { slot: NodeSlotId, damage: PaintDamage },
    /// A box and its paint subtree paint again.
    SubtreeRepaint(NodeSlotId),
    /// Every box paints again.
    FullRepaint,
    /// Whether recordings leave a trace for the host, for tests.
    RecordingTraceEnabled(bool),
    /// The document's SVG paint resources changed: they are synchronized before the next recording.
    SvgPaintResourcesChanged,
    /// A rendering update's compositor animations start over.
    CompositorAnimationUpdateBegun,
    /// The SVG-as-image renders the next recording looks up what it paints in.
    VectorImageDisplayLists(std::sync::Arc<crate::painting::record::vector_images::VectorImageDisplayLists>),
    /// Compositor animations an effect published in the rendering update.
    CompositorAnimations(Vec<crate::painting::visual_animation::VisualAnimation>),
}

impl PaintChange {
    /// Whether applying the change can alter what the rows the owner publishes answer the document thread: the
    /// paintable rows, the scroll offsets and image map areas beside them, and the navigable a container box hosts.
    /// Damage, marks for the next visual context update and what only the next recording reads alter none.
    pub(crate) fn alters_published_rows(&self) -> bool {
        match self {
            Self::ScrollbarEnlarged { .. } | Self::ImageMapAreas { .. } | Self::ScrollOffset { .. } => true,
            Self::ReplacedPaintFacts { facts, .. } => {
                matches!(
                    facts,
                    crate::painting::replaced_paint_facts::ReplacedPaintFacts::NavigableContainer(_)
                )
            }
            Self::NearestSelfPaintingInlineRepaint(_)
            | Self::Selection(_)
            | Self::SelectionCleared { .. }
            | Self::ScrollableOverflowRecalculationCountReset
            | Self::VisualContextBoxDirty { .. }
            | Self::VisualContextFullRebuild(_)
            | Self::ScrollStateInvalidated
            | Self::LayerImagePaintFacts { .. }
            | Self::VisualContextTreeInputs(_)
            | Self::PaintCacheInvalidated { .. }
            | Self::Repaint { .. }
            | Self::SubtreeRepaint(_)
            | Self::FullRepaint
            | Self::RecordingTraceEnabled(_)
            | Self::SvgPaintResourcesChanged
            | Self::CompositorAnimationUpdateBegun
            | Self::VectorImageDisplayLists(_)
            | Self::CompositorAnimations(_) => false,
        }
    }

    pub(crate) fn apply(self, arena: &mut LayoutNodeArena) {
        match self {
            Self::ScrollbarEnlarged {
                slot,
                direction,
                enlarged,
            } => {
                let mut rows = arena.paintable_rows_mut();
                if !rows.paintable_row_is_populated(slot) {
                    return;
                }
                let flag = match direction {
                    ScrollDirection::Horizontal => PaintableFlag::HorizontalScrollbarEnlarged,
                    ScrollDirection::Vertical => PaintableFlag::VerticalScrollbarEnlarged,
                };
                if rows.paintable_data(slot).has_flag(flag) == enlarged {
                    return;
                }
                rows.paintable_data_mut(slot).set_flag(flag, enlarged);
                rows.push_paint_damage(slot, PaintDamage::DRAW_OVERLAY | PaintDamage::HIT_OVERLAY);
            }
            Self::NearestSelfPaintingInlineRepaint(node) => {
                if !arena.slot_is_live(node) {
                    return;
                }
                if let Some(ancestor) =
                    crate::painting::fragment_ownership::nearest_self_painting_inline_box(&arena.paintable_rows(), node)
                {
                    arena.push_paint_damage(ancestor, PaintDamage::ALL_DRAW | PaintDamage::ALL_HIT);
                }
            }
            Self::Selection(snapshot) => snapshot.apply(arena),
            Self::SelectionCleared { viewport } => {
                if arena.paintable_row_is_populated(viewport) {
                    crate::painting::selection::clear(&mut arena.paintable_rows_mut(), viewport);
                }
            }
            Self::ScrollableOverflowRecalculationCountReset => arena.scrollable_overflow.recalculations.set(0),
            Self::VisualContextBoxDirty { slot, kind } => {
                if arena.paintable_row_is_populated(slot) {
                    arena.note_visual_context_box_dirty(slot, kind);
                }
            }
            Self::VisualContextFullRebuild(reason) => arena.request_full_visual_context_rebuild(reason),
            Self::ScrollStateInvalidated => {
                arena
                    .paint_state()
                    .borrow_mut()
                    .visual_context
                    .needs_to_refresh_scroll_state = true;
            }
            Self::ReplacedPaintFacts {
                slot,
                facts,
                damage_when_changed,
            } => {
                if arena.set_replaced_paint_facts(slot, facts)
                    && !damage_when_changed.is_empty()
                    && arena.paintable_row_is_populated(slot)
                {
                    arena.push_paint_damage(slot, damage_when_changed);
                }
            }
            Self::LayerImagePaintFacts { slot, entries } => {
                arena.set_layer_image_paint_facts(slot, entries);
            }
            Self::ImageMapAreas { slot, areas } => arena.image_map_areas().publish(slot, areas),
            Self::ScrollOffset {
                slot,
                offset,
                dom_target_stores_offset,
            } => {
                if !arena.slot_is_live(slot) {
                    return;
                }
                arena.set_node_flag(
                    slot,
                    crate::layout::node_data::NodeFlag::HasScrollOffset,
                    dom_target_stores_offset,
                );
                arena.scroll_offsets().publish(slot, offset);
            }
            Self::VisualContextTreeInputs(inputs) => arena.publish_visual_context_tree_inputs(inputs),
            Self::PaintCacheInvalidated {
                slot,
                propagated_text_decorations,
            } => {
                if !arena.slot_is_live(slot) {
                    return;
                }
                if propagated_text_decorations {
                    arena.push_propagated_text_decoration_damage(slot);
                } else {
                    arena.push_paint_damage(slot, PaintDamage::ALL_DRAW | PaintDamage::ALL_HIT);
                }
            }
            Self::Repaint { slot, damage } => {
                if arena.slot_is_live(slot) {
                    arena.push_paint_damage_for_repaint(slot, damage);
                }
            }
            Self::SubtreeRepaint(slot) => {
                if arena.slot_is_live(slot) {
                    arena.push_paint_damage_to_paint_subtree(slot, PaintDamage::ALL_PRODUCERS);
                }
            }
            Self::FullRepaint => arena.push_all_paint_damage(),
            Self::RecordingTraceEnabled(enabled) => arena.paint_state().borrow_mut().trace_recordings = enabled,
            Self::SvgPaintResourcesChanged => {
                arena.svg_paint_resources().note_changed();
            }
            Self::CompositorAnimationUpdateBegun => {
                arena
                    .paint_state()
                    .borrow_mut()
                    .visual_context
                    .pending_compositor_animations
                    .clear();
            }
            Self::VectorImageDisplayLists(resolved) => {
                arena.paint_state().borrow_mut().vector_image_display_lists = resolved;
            }
            Self::CompositorAnimations(animations) => {
                arena
                    .paint_state()
                    .borrow_mut()
                    .visual_context
                    .pending_compositor_animations
                    .extend(animations);
            }
        }
    }
}

/// Sends `change` to the owner of the document whose arena `arena` names. A host call a stage makes is the owner's
/// already: the change applies to the arena the stage holds, in place.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, on the document thread or inside a unit the owner runs
/// for it.
pub(crate) unsafe fn send(arena: *mut c_void, change: PaintChange) -> Option<crate::render_owner::ChangeSeq> {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
    // The owner holds no state of an arena of no document (a unit test's).
    if !document.is_valid()
        || crate::stage_thread::running_inside_stage()
        || crate::stage_thread::owner_work_runs_here()
    {
        // SAFETY: The unit the owner runs holds the arena, and its document thread waits for it; or the document
        // thread does the owner's work; or the arena is the calling thread's alone.
        let arena = crate::render_owner::do_owner_work_here(|owner| unsafe {
            &mut *crate::layout::ArenaHandle::held_by_waiting_thread(owner, arena)
        })
        .arena_mut();
        change.apply(arena);
        // The unit publishes the rows as it ends; the document thread reads them next.
        if !crate::stage_thread::running_inside_stage() {
            arena.publish_rows();
        }
        return None;
    }
    Some(crate::render_owner::send_arena_change(
        document,
        crate::render_owner::ArenaChange::Paint(change),
    ))
}
