/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

/// What a commit message tells the document.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiCommitMessageKind {
    /// The node is a navigable container whose viewport committed at a new size.
    NavigableContainerViewportCommitted,
    /// The node is an inline box that reached atomic-inline layout without line box fragments.
    UnexpectedFragmentedInline,
    /// The node is an SVG resource - a `<mask>`, `<clipPath>` or `<pattern>` - whose content the
    /// tree build laid out under the graphics element `other_style_node` names. The resource
    /// outlives that box, so removing it has to rebuild the subtree the box sits in.
    SvgResourceReferenced,
    /// A top layer member was reached with no box and nothing scheduled to rebuild it, so the
    /// document has to run another top layer zone pass. This one is about the document itself.
    TopLayerZoneRebuildNeeded,
    /// The node is the element a pseudo-element box escaped its rebuild root under, so its layout
    /// tree has to be built again.
    LayoutTreeRebuildRequested,
    /// A pass reached the web font face `pending_face` names while it waits on its load. A pass
    /// cannot start the load itself: the fetch, the font-display timer and the load-event delayer
    /// are all document state. This one is about the document itself.
    PendingFontFaceWanted,
    /// A tree build placed a new viewport in place of the one before, whose paint state went with
    /// it: the document gives the new tree a new paint state once it has taken in what the build
    /// found out before this. This one is about the document itself.
    LayoutTreeReplaced,
}

/// One thing the render side has to tell the document. The node it is about is named by the style
/// node the style tree gave it, with 0 for the document; no pointer crosses the boundary.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiCommitMessage {
    pub style_node: u32,
    /// A second node the message names, for the kinds that are about a pair. Zero otherwise.
    pub other_style_node: u32,
    pub kind: FfiCommitMessageKind,
    /// The face a `PendingFontFaceWanted` message names, and whether the document has been offered
    /// it once before without finding it. Zero and false otherwise.
    pub pending_face: u64,
    pub pending_face_has_been_retried: bool,
}

/// Host notifications contain no arena borrows. Dispatch them only after the
/// mutation phase returns, since C++ can reenter Rust to read or update paint state.
pub(crate) struct CommitNotifications {
    row_resets: Vec<crate::painting::paintable_rows::PaintableRowReset>,
    messages: Vec<FfiCommitMessage>,
    /// The commit gave a navigable container viewport another size: the navigable it hosts lays
    /// itself out again at that size, and paints after the container's document.
    resized_a_hosted_navigable: bool,
}

impl CommitNotifications {
    /// Whether the commit resized a navigable its document hosts, whose frame at the new size is
    /// painted after the container's document is.
    pub(crate) fn resized_a_hosted_navigable(&self) -> bool {
        self.resized_a_hosted_navigable
    }

    /// Whether telling the host these leaves it no style or layout work to do: the messages ask for
    /// no rebuild and no top layer pass. A navigable container's committed viewport sizes the
    /// navigable it hosts, whose document lays itself out; the container's document has nothing
    /// more to do for it.
    pub(crate) fn leave_the_host_no_work(&self) -> bool {
        self.messages.iter().all(|message| {
            matches!(
                message.kind,
                FfiCommitMessageKind::NavigableContainerViewportCommitted
                    | FfiCommitMessageKind::PendingFontFaceWanted
                    | FfiCommitMessageKind::UnexpectedFragmentedInline
                    | FfiCommitMessageKind::SvgResourceReferenced
            )
        })
    }

    /// # Safety
    ///
    /// The host must keep the document and node shells alive until these synchronous
    /// notifications return. No mutable arena borrow may be active. `viewport_row` is the row the
    /// document's viewport is bound to.
    pub(crate) unsafe fn notify_host(
        self,
        main_thread: &crate::stage::MainThread,
        host: &LayoutHost,
        viewport_row: NodeSlotId,
    ) {
        for reset in self.row_resets {
            super::tree_build_seal::note_host_call("paintable_row_reset");
            reset.invoke_callback_on_main_thread(main_thread, viewport_row);
        }
        if !self.messages.is_empty() {
            super::tree_build_seal::note_host_call("deliver_commit_messages");
            unsafe { host.deliver_commit_messages(main_thread, &self.messages) };
        }
    }
}

fn commit_subtree(
    node: Node,
    paintables: &mut crate::painting::paintable_build::PaintableCommit<'_>,
    links_by_slot: &HashMap<u32, &FragmentLink>,
    pass_fragments: &fragment_tree::CompletedPassFragments,
    enclosing_line_root_changes: crate::painting::paintable_build::LineRootChanges,
) {
    let slot_index = node.slot_index();
    let entry = links_by_slot.get(&slot_index).copied();
    let reuses_committed_subtree = pass_fragments.subtree_was_reused(slot_index);
    debug_assert!(!reuses_committed_subtree || entry.is_some());
    if !reuses_committed_subtree {
        paintables.arena().forget_committed_out_of_flow_facts(node);
    }
    if let Some(inputs) = entry.and_then(|link| link.abspos_layout_inputs.as_ref()) {
        paintables.arena().note_committed_out_of_flow_box(node, inputs);
    }
    let prepared = paintables.prepare_node(
        node,
        entry.is_some(),
        reuses_committed_subtree,
        enclosing_line_root_changes,
    );

    let mut has_pending_inline_box_geometry = false;
    let mut line_root_changes_for_children = enclosing_line_root_changes;
    let mut laid_out_content_size = None;
    if let Some(link) = entry
        && prepared.has_paintable_row
    {
        let fragment = &link.fragment;
        laid_out_content_size = Some(FfiCssPixelSize {
            width: fragment.content_inline_size,
            height: fragment.content_block_size,
        });
        debug_assert!(
            fragment.computed_svg_path.is_some()
                || !matches!(
                    paintables.arena().data(node).kind.get(),
                    NodeKind::SVGGeometryBox | NodeKind::SVGTextBox | NodeKind::SVGTextPathBox
                ),
            "committed path-like fragment carries no computed SVG path"
        );
        let replaced = paintables.replace_committed_fragment_link(
            node,
            link,
            reuses_committed_subtree,
            enclosing_line_root_changes,
            prepared.previous_offset,
        );
        if fragment.line_data.is_some() {
            line_root_changes_for_children = replaced.line_root_changes;
        }
        if prepared.row_existed_before_this_commit
            && let Some((old_content_size, new_content_size)) = replaced.content_size_change
            && crate::layout::node_facts::node_style_view(paintables.arena().data(node)).is_some_and(|style| {
                content_size_change_affects_container_queries(style, old_content_size, new_content_size)
            })
            && let Some(style_node) = paintables.arena().commit_message_style_node(node)
        {
            paintables.arena().record_size_container_content_size_change(style_node);
        }

        if !reuses_committed_subtree && let Some(line_data) = &fragment.line_data {
            has_pending_inline_box_geometry = paintables.set_line_data(node, line_data);
        }
    }

    if entry.is_none() && prepared.has_paintable_row {
        paintables.schedule_scrollable_overflow_recalculation(node);
    }

    paintables
        .arena()
        .publish_layout_style_snapshot_geometry(node, laid_out_content_size);

    paintables.stamp_containing_block(node, entry);
    if reuses_committed_subtree {
        return;
    }
    paintables.arena().refresh_paint_order_inputs(node);

    let mut child = paintables.arena().data(node).first_child.get();
    while !child.is_invalid() {
        let next = paintables.arena().data(child).next_sibling.get();
        commit_subtree(
            child,
            &mut *paintables,
            links_by_slot,
            pass_fragments,
            line_root_changes_for_children,
        );
        child = next;
    }

    if has_pending_inline_box_geometry {
        // Inline box geometry unites this block's piece rects with the box
        // models of its descendant inline paintables, which exist only now
        // that the whole subtree has committed.
        paintables.assign_inline_box_geometry(node);
    }
}

fn content_size_change_affects_container_queries(
    style: crate::css::computed_value_views::ComputedValuesView<'_>,
    old_size: FfiCssPixelSize,
    new_size: FfiCssPixelSize,
) -> bool {
    let box_values = style.box_values();
    if box_values.is_size_container {
        return old_size.width != new_size.width || old_size.height != new_size.height;
    }
    if !box_values.is_inline_size_container {
        return false;
    }
    if style.writing_mode() == crate::css::css_enums::writing_mode::HORIZONTAL_TB {
        old_size.width != new_size.width
    } else {
        old_size.height != new_size.height
    }
}

pub(crate) fn commit_replacing(
    root: Node,
    arena: &mut LayoutNodeArena,
    pass_fragments: &fragment_tree::CompletedPassFragments,
) -> CommitNotifications {
    let links_by_slot = pass_fragments.links_by_slot();
    arena.release_published_paintable_rows();
    arena.note_layout_commit();
    arena.begin_layout_style_snapshot_commit();
    let mut paintables = crate::painting::paintable_build::PaintableCommit::new(arena, root);
    paintables.begin_commit();
    // What the pass itself found out comes before what committing it finds out.
    let mut messages = paintables.arena().take_messages_reported_during_pass();
    commit_subtree(
        root,
        &mut paintables,
        &links_by_slot,
        pass_fragments,
        Default::default(),
    );
    paintables.discard_absolute_rects_memoized_during_commit();
    for (viewport, _) in paintables.committed_navigable_container_viewports() {
        if let Some(style_node) = paintables.arena().commit_message_style_node(*viewport) {
            messages.push(FfiCommitMessage {
                style_node,
                other_style_node: 0,
                kind: FfiCommitMessageKind::NavigableContainerViewportCommitted,
                pending_face: 0,
                pending_face_has_been_retried: false,
            });
        }
    }
    let resized_a_hosted_navigable = paintables.resized_a_navigable_container_viewport();
    // The rows are published when the main side next reads them, or when a recording is submitted:
    // what derives from the commit before either writes them in place.
    paintables.arena().finish_layout_style_snapshot_commit();
    CommitNotifications {
        row_resets: paintables.take_row_reset_notifications(),
        messages,
        resized_a_hosted_navigable,
    }
}
