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
use super::layout_node_arena::{BoxPresenceHost, ShellFactory, ShellStyleChangedHost};
use super::node_data::NodeSlotId;
use super::update_layout::LayoutUpdateHost;
use crate::css::style::fast_hash::FastMap as HashMap;
use crate::painting::host::GeometryHostCallbacks;
use crate::painting::paintable_rows::ChromeStateCallback;
use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::c_void;

#[derive(Default)]
pub(crate) struct HostTables {
    pub(super) layout_host: Cell<Option<FfiLayoutHostCallbacks>>,
    pub(super) layout_update_host: Cell<Option<LayoutUpdateHost>>,
    /// The flight the document's layout update readied, until the document has sealed what its
    /// recording reads and submits it.
    pub(super) prepared_flight: RefCell<Option<super::update_layout::PreparedFlight>>,
    /// The document's layout update while it waits for the document to run the style of the round
    /// it started.
    pub(super) parked_layout_update: RefCell<Option<super::update_layout::ParkedLayoutUpdate>>,
    pub(super) shell_factory: Cell<Option<ShellFactory>>,
    pub(super) box_presence_host: Cell<Option<BoxPresenceHost>>,
    pub(super) shell_style_changed_host: Cell<Option<ShellStyleChangedHost>>,
    pub(crate) geometry_host: Cell<Option<GeometryHostCallbacks>>,
    pub(crate) chrome_state_callback: Cell<Option<ChromeStateCallback>>,
    /// The image provider each row that owns one owns. The arena knows which rows those are.
    pub(super) owned_image_providers: RefCell<HashMap<NodeSlotId, *mut c_void>>,
    /// The image observer set each row that holds one holds. The arena knows which rows those are.
    pub(super) image_observer_sets: RefCell<HashMap<NodeSlotId, *mut c_void>>,
    /// Observer sets the arena has handed back that a newer set displaced before the handback was
    /// paid, in the order they were displaced.
    pub(super) image_observer_sets_owed: RefCell<Vec<(NodeSlotId, *mut c_void)>>,
    /// Whether the document's invalidation journal holds marks the render side has not taken yet.
    /// The document only reports it while the main-side access census counts.
    pub(super) invalidation_journal_pending: Cell<bool>,
    /// How the host names a node a layout trace mentions, set when tracing begins.
    pub(super) layout_trace_describe_node: Cell<Option<super::trace::DescribeNode>>,
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
    /// What each row of the style batch a flight applied marked of its element's layout nodes, by style node, packed
    /// as an `FfiStyleInvalidationField` word, with the record it installed, while the host installs the batch.
    flight_style_damages: RefCell<HashMap<crate::css::style::tree::StyleNodeID, (u32, u64)>>,
}

impl HostTables {
    /// Notes that the document runs a layout update. A document runs one at a time.
    pub(crate) fn begin_layout_update(&self) {
        let was_running = self.layout_update_is_running.replace(true);
        debug_assert!(!was_running, "a layout update is already running");
    }

    /// Notes that the document's layout update is over.
    pub(crate) fn end_layout_update(&self) {
        let was_running = self.layout_update_is_running.replace(false);
        debug_assert!(was_running, "no layout update is running");
    }

    /// Whether the document runs a layout update.
    pub(crate) fn layout_update_is_running(&self) -> bool {
        self.layout_update_is_running.get()
    }

    /// Holds what the rows of the style batch a flight applied marked, for the host to read as it installs the
    /// batch; what the host does not take goes with the next.
    pub(crate) fn hold_flight_style_damages(&self, damages: HashMap<crate::css::style::tree::StyleNodeID, (u32, u64)>) {
        *self.flight_style_damages.borrow_mut() = damages;
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

    /// Whether the document traces its layout, and has the owners of the trace lines named once a frame is over.
    pub(crate) fn traces_layout(&self) -> bool {
        self.layout_trace_describe_node.get().is_some()
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

    /// The layout scratch of the arena `handle` names.
    ///
    /// # Safety
    ///
    /// As for [`HostTables::from_handle`].
    pub(crate) unsafe fn layout_scratch_of<'a>(handle: *mut c_void) -> &'a mut super::LayoutScratch {
        assert!(!handle.is_null(), "layout node arena handle is null");
        // SAFETY: Guaranteed by the caller. The projection does not borrow the arena beside it.
        unsafe { &mut *std::ptr::addr_of_mut!((*handle.cast::<ArenaHandle>()).layout_scratch) }
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
    /// to the state.
    ///
    /// # Safety
    ///
    /// `handle` must be a live handle from `layout_arena_create`, and nothing else may reach the state while the job
    /// runs.
    pub(crate) unsafe fn held_by_waiting_thread(handle: *mut c_void) -> *mut Self {
        handle.cast()
    }
}
