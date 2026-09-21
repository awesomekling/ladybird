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
}

impl HostTables {
    /// The host tables of the arena `handle` names.
    ///
    /// # Safety
    ///
    /// `handle` must come from `layout_arena_create` and stay live for `'a`.
    pub(crate) unsafe fn from_handle<'a>(handle: *mut c_void) -> &'a Self {
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
}

// A handle is also a pointer to its arena.
const _: () = assert!(std::mem::offset_of!(ArenaHandle, arena) == 0);

impl ArenaHandle {
    pub(crate) fn new() -> Self {
        Self {
            arena: LayoutNodeArena::new(),
            host_tables: HostTables::default(),
        }
    }

    pub(crate) fn arena(&self) -> &LayoutNodeArena {
        &self.arena
    }
}
