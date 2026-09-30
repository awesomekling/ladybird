/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the host registers with a document's layout arena: the callback tables it answers
//! through. None of it is data a stage reads, so it is kept beside the arena rather than in it,
//! and it is reached through the main thread capability, which entries mint from the handle C++
//! holds. A stage borrows the arena and is never handed the capability, so it cannot reach them.

use super::LayoutNodeArena;
use super::formatting_context::FfiLayoutHostCallbacks;
use super::layout_node_arena::BoxPresenceHost;
use super::node_data::NodeSlotId;
use super::update_layout::LayoutUpdateHost;
use crate::css::style::fast_hash::{FastMap as HashMap, FastSet as HashSet};
use crate::painting::host::GeometryHostCallbacks;
use crate::painting::paintable_rows::ChromeStateCallback;
use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::c_void;

#[derive(Default)]
pub(crate) struct HostTables {
    pub(super) layout_host: Cell<Option<FfiLayoutHostCallbacks>>,
    /// While the document traces its layout: how the host names the DOM nodes the trace mentions, and the names it
    /// gave the nodes the layout commits taken in so far named, while the nodes were live.
    pub(super) layout_trace_names: RefCell<Option<super::trace::LayoutTraceNames>>,
    pub(super) layout_update_host: Cell<Option<LayoutUpdateHost>>,
    /// The flight the document's layout update readied, until the document has sealed what its
    /// recording reads and submits it.
    pub(super) prepared_flight: RefCell<Option<super::update_layout::LayoutPassJob>>,
    pub(super) box_presence_host: Cell<Option<BoxPresenceHost>>,
    pub(crate) geometry_host: Cell<Option<GeometryHostCallbacks>>,
    pub(crate) chrome_state_callback: Cell<Option<ChromeStateCallback>>,
    /// The image provider the host gave each row that owns one. The owner is told which rows those are, and hands
    /// each back as its row lets go of it.
    pub(super) owned_image_providers: RefCell<HashMap<NodeSlotId, *mut c_void>>,
    /// The image observer set the host gave each row that holds one, told to the owner and handed back as for
    /// [`Self::owned_image_providers`].
    pub(super) image_observer_sets: RefCell<HashMap<NodeSlotId, *mut c_void>>,
    /// The compositor animation frames the rendering update gave each box, by kind, which only it chooses.
    pub(super) compositor_animation_frames: RefCell<HashMap<NodeSlotId, u8>>,
    /// The image boxes the tree builds of the layout update in progress stamped to own their image's provider, which
    /// the host has not handed it yet: until the frame is over, such a box shows no image.
    pub(super) image_boxes_awaiting_owned_provider: RefCell<HashSet<NodeSlotId>>,
    /// The number of the last change the document thread sent the owner that lays a node out again.
    pub(super) last_relayout_change_sent: Cell<Option<crate::render_owner::ChangeSeq>>,
    /// The number of the last change the document thread sent the owner to build the whole layout tree again, or not,
    /// and which.
    pub(super) full_layout_tree_update_sent: Cell<Option<(crate::render_owner::ChangeSeq, bool)>>,
    /// The generation of the document's render state, which retiring it moves on. See
    /// [`super::frame_retirement`].
    pub(super) frame_generation: Cell<u64>,
    /// The document's layout tree update marks, which the arena reaches for the tree build they are lent to. See
    /// [`super::tree_update_marks`].
    pub(super) layout_tree_update_marks: RefCell<super::tree_update_marks::LayoutTreeUpdateMarks>,
    /// Whether the tree build holds the marks, for its walk.
    pub(super) layout_tree_update_marks_are_lent: Cell<bool>,
    /// What the document thread wrote to the marks beside the frame they are lent to, in order,
    /// written once the frame has handed them back.
    pub(super) layout_tree_update_mark_writes_waiting_for_frame:
        RefCell<Vec<super::tree_update_marks::MarkWriteWaitingForFrame>>,
    /// Whether the document runs a layout update, between `layout_arena_begin_update_layout` and its end.
    layout_update_is_running: Cell<bool>,
    /// The rows the document thread adopted from the display ticks of its clock, which it reads until the arena
    /// publishes later ones (see [`super::row_reads::RowSnapshot::published`]).
    pub(crate) adopted_rows: super::row_reads::RowSnapshotSlot,
    /// What each row of the style batch a flight applied marked of its element's layout nodes, by style node, packed
    /// as an `FfiStyleInvalidationField` word, with the record it installed, while the host installs the batch.
    flight_style_damages: RefCell<HashMap<crate::css::style::tree::StyleNodeID, (u32, u64)>>,
    /// The document's style engine, which the host linked the arena to, for the document thread to lend
    /// and ask where it is without reaching the arena.
    pub(super) style_engine: Cell<Option<crate::css::style::StyleEngineHandle>>,
    /// What the document thread wrote to rows that the owner has not taken in yet.
    pub(super) rows_sent_ahead: RefCell<super::row_reads::RowsSentAhead>,
}

impl HostTables {
    /// The document's style engine, or null before the host links the arena to one.
    pub(crate) fn style_engine(&self) -> crate::css::style::StyleEngineHandle {
        self.style_engine
            .get()
            .unwrap_or_else(crate::css::style::StyleEngineHandle::null)
    }

    /// Notes that the document runs a layout update. A document runs one at a time.
    pub(crate) fn begin_layout_update(&self) {
        let was_running = self.layout_update_is_running.replace(true);
        debug_assert!(!was_running, "a layout update is already running");
    }

    /// Gives `slot` the image observer set `observers`, or none, and answers the set it held (or null).
    pub(crate) fn replace_image_observers(&self, slot: NodeSlotId, observers: *mut c_void) -> *mut c_void {
        let mut sets = self.image_observer_sets.borrow_mut();
        let previous = if observers.is_null() {
            sets.remove(&slot)
        } else {
            sets.insert(slot, observers)
        };
        previous.unwrap_or(std::ptr::null_mut())
    }

    /// Notes whether the rendering update gave `row` the compositor animation frame of `kind`.
    pub(crate) fn set_compositor_animation_frame(
        &self,
        row: NodeSlotId,
        kind: super::node_data::CompositorAnimationFrameKind,
        value: bool,
    ) {
        let mut frames = self.compositor_animation_frames.borrow_mut();
        let kinds = frames.get(&row).copied().unwrap_or(0);
        let kinds = if value {
            kinds | kind as u8
        } else {
            kinds & !(kind as u8)
        };
        if kinds == 0 {
            frames.remove(&row);
        } else {
            frames.insert(row, kinds);
        }
    }

    /// Notes that the document's layout update is over.
    pub(crate) fn end_layout_update(&self) {
        let was_running = self.layout_update_is_running.replace(false);
        debug_assert!(was_running, "no layout update is running");
        // The frame's end handed every box awaiting its provider the provider, or its row is gone.
        self.image_boxes_awaiting_owned_provider.borrow_mut().clear();
    }

    /// Whether the document runs a layout update.
    pub(crate) fn layout_update_is_running(&self) -> bool {
        self.layout_update_is_running.get()
    }

    /// Takes what the rows of the style batches the render owner applied marked that the host did not take as it
    /// installed them.
    pub(crate) fn take_flight_style_damages(&self) -> HashMap<crate::css::style::tree::StyleNodeID, (u32, u64)> {
        self.flight_style_damages.take()
    }

    /// Holds what the rows of a style batch the render owner applied marked, beside what the batches it applied
    /// earlier in the style update left, for the host to read as it installs the batch; what the host does not take
    /// goes as the style update ends.
    pub(crate) fn hold_owner_style_damages(&self, damages: HashMap<crate::css::style::tree::StyleNodeID, (u32, u64)>) {
        self.flight_style_damages.borrow_mut().extend(damages);
    }

    /// Takes what the flight marked of the layout nodes of `style_node`'s element as it applied the style row the
    /// host installs now, or `None` if it applied none for it.
    pub(crate) fn take_flight_style_damage(&self, style_node: crate::css::style::tree::StyleNodeID) -> Option<u32> {
        self.flight_style_damages
            .borrow_mut()
            .remove(&style_node)
            .map(|(damage, _)| damage)
    }

    /// What the flight marked of the layout nodes of `style_node`'s element, which the host reads before it takes it,
    /// if the flight installed `style_record` there.
    pub(crate) fn flight_style_damage(
        &self,
        style_node: crate::css::style::tree::StyleNodeID,
        style_record: u64,
    ) -> Option<u32> {
        self.flight_style_damages
            .borrow()
            .get(&style_node)
            .filter(|(_, installed)| *installed == style_record)
            .map(|(damage, _)| *damage)
    }

    /// The host tables of the arena `handle` names.
    ///
    /// # Safety
    ///
    /// `handle` must come from `layout_arena_create` and stay live for `'a`.
    #[track_caller]
    pub(crate) unsafe fn from_handle<'a>(handle: *mut c_void) -> &'a Self {
        assert!(!handle.is_null(), "layout node arena handle is null");
        crate::stage_thread::join_frame_in_flight(handle);
        // SAFETY: Guaranteed by the caller. The projection does not borrow the arena beside it.
        unsafe { &*std::ptr::addr_of!((*handle.cast::<ArenaHandle>()).host_tables) }
    }

    /// The host tables of the arena `handle` names, without waiting for a frame in flight. No stage
    /// reaches the host tables, so the document thread may use them beside one.
    ///
    /// # Safety
    ///
    /// As for [`Self::from_handle`], on the document thread.
    pub(crate) unsafe fn beside_frame<'a>(handle: *mut c_void) -> &'a Self {
        assert!(!handle.is_null(), "layout node arena handle is null");
        // SAFETY: Guaranteed by the caller. The projection does not borrow the arena beside it.
        unsafe { &*std::ptr::addr_of!((*handle.cast::<ArenaHandle>()).host_tables) }
    }
}

/// What `layout_arena_create` hands C++: the arena, first, so a handle is also a pointer to it, and
/// the host tables beside it.
#[repr(C)]
pub(crate) struct ArenaHandle {
    arena: LayoutNodeArena,
    host_tables: HostTables,
    layout_scratch: super::LayoutScratch,
}

// A handle is also a pointer to its arena.
const _: () = assert!(std::mem::offset_of!(ArenaHandle, arena) == 0);

impl ArenaHandle {
    /// An arena of no document's render state, owned by the calling thread.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::new_for(crate::render_owner::DocumentId::default(), std::thread::current().id())
    }

    /// The arena of the render state of `document`, which the document thread `document_thread` acts for.
    pub(crate) fn new_for(document: crate::render_owner::DocumentId, document_thread: std::thread::ThreadId) -> Self {
        let mut arena = LayoutNodeArena::new_for(document_thread);
        arena.adopt(document, document_thread);
        Self {
            arena,
            host_tables: HostTables::default(),
            layout_scratch: super::LayoutScratch::default(),
        }
    }

    /// The document whose render state holds the arena `handle` names.
    ///
    /// # Safety
    ///
    /// As for [`HostTables::from_handle`].
    pub(crate) unsafe fn document_of(handle: *const c_void) -> crate::render_owner::DocumentId {
        assert!(!handle.is_null(), "layout node arena handle is null");
        // SAFETY: Guaranteed by the caller. The projection does not borrow the arena beside it.
        unsafe { *std::ptr::addr_of!((*handle.cast::<ArenaHandle>()).arena.document) }
    }

    /// Makes the arena the render state of `document`'s, which the document thread `document_thread` acts for.
    pub(crate) fn adopt(&mut self, document: crate::render_owner::DocumentId, document_thread: std::thread::ThreadId) {
        self.arena.adopt(document, document_thread);
        self.arena
            .link_layout_tree_update_marks(&self.host_tables.layout_tree_update_marks);
    }

    pub(crate) fn arena_mut(&mut self) -> &mut LayoutNodeArena {
        &mut self.arena
    }

    /// For a unit the document thread waits for, which publishes before it answers: lets go of the rows the arena
    /// published last ([`LayoutNodeArena::let_go_of_rows_while_document_thread_waits`]), and of those the thread
    /// adopted from display ticks where the arena's are later, which the thread does not read again.
    pub(crate) fn let_go_of_rows_while_document_thread_waits(&mut self) {
        self.arena.let_go_of_rows_while_document_thread_waits();
        self.host_tables
            .adopted_rows
            .let_go_if_earlier_than(&self.arena.published_rows);
    }

    pub(crate) fn arena(&self) -> &LayoutNodeArena {
        &self.arena
    }

    /// The arena and its layout scratch, which a layout pass reads and writes side by side.
    pub(crate) fn arena_and_scratch(&mut self) -> (&mut LayoutNodeArena, &mut super::LayoutScratch) {
        (&mut self.arena, &mut self.layout_scratch)
    }

    /// The render state of the document the handle `handle` a document thread holds names, for a job of the document
    /// that runs on that thread, or beside it, rather than on the render owner, which hands the jobs it runs their
    /// state: a job the owner cannot take (a test holds the run it would queue behind), a debug path that runs a unit
    /// in place, or a stage the document thread submitted outside a rendering update. The one way from such a handle
    /// to the state, which only the `owner` takes, so a path of the main thread's cannot.
    ///
    /// # Safety
    ///
    /// `handle` must be a live handle from `layout_arena_create`, and nothing else may reach the state while the job
    /// runs.
    pub(crate) unsafe fn held_by_waiting_thread(_owner: &crate::render_owner::Owner, handle: *mut c_void) -> *mut Self {
        handle.cast()
    }
}
