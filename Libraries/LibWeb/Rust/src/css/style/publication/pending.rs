/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::{EngineComputedRecordContinuation, ParentInputsMoved};
use crate::css::style::tree::StyleNodeID;

/// One canonical record computation suspended on a font cache miss. Its descendants wait for the
/// completed record they inherit from, while records outside its subtree may continue this pass.
pub(in crate::css::style) struct ParkedEngineComputedRecord {
    pub(in crate::css::style) published_index: usize,
    pub(in crate::css::style) subtree_root: StyleNodeID,
    pub(in crate::css::style) parent_inputs: ParentInputsMoved,
    pub(in crate::css::style) continuation: EngineComputedRecordContinuation,
}
