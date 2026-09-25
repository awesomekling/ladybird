/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TreeBuilder.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Selection/Selection.h>

namespace Web::DOM {

static Layout::RustFFI::FfiUtf16View ffi_utf16_view(Utf16View view)
{
    return {
        .ascii = view.has_ascii_storage() ? reinterpret_cast<u8 const*>(view.ascii_span().data()) : nullptr,
        .utf16 = view.has_ascii_storage() ? nullptr : reinterpret_cast<u16 const*>(view.utf16_span().data()),
        .length = view.length_in_code_units(),
    };
}

// The document-side steps of the layout update, which the Rust loop drives through this table.
Layout::RustFFI::FfiLayoutUpdateHostCallbacks Document::layout_update_host_callbacks()
{
    return {
        .context = this,
        .update_style = [](void* context) { static_cast<Document*>(context)->update_style(); },
        .process_pending_list_item_renumbers = [](void* context) { static_cast<Document*>(context)->process_pending_list_item_renumbers(); },
        .process_pending_top_layer_layout_changes = [](void* context) { static_cast<Document*>(context)->process_pending_top_layer_layout_changes(); },
        .document_facts = [](void* context) -> Layout::RustFFI::FfiLayoutUpdateDocumentFacts {
            auto& document = *static_cast<Document*>(context);
            auto navigable = document.navigable();
            bool document_is_active = navigable && navigable->active_document().ptr() == &document;
            auto viewport_rect = document_is_active ? navigable->viewport_rect() : CSSPixelRect {};
            return {
                .document_is_active = document_is_active,
                .document_needs_layout_tree_build = document.needs_layout_tree_update() || document.child_needs_layout_tree_update(),
                .container_query_evaluation_is_pending = document.has_size_containers_needing_evaluation_after_layout(),
                .top_layer_work_pending = document.m_top_layer_needs_layout_zone_rebuild || !document.m_elements_with_pending_top_layer_membership_change.is_empty(),
                .should_collect_devtools_layout_data = document.page().client().has_active_devtools_client(),
                .document_in_quirks_mode = document.in_quirks_mode(),
                .viewport_inline_size_raw = viewport_rect.width().raw_value(),
                .viewport_block_size_raw = viewport_rect.height().raw_value(),
            }; },
        .needs_style_update_after_layout = [](void* context) -> bool { return static_cast<Document*>(context)->needs_style_update_after_layout(); },
        .prepare_for_rendering = [](void* context) { static_cast<Document*>(context)->prepare_for_rendering(); },
        .prepare_layout_tree_build = [](void* context) -> u32 { return static_cast<Document*>(context)->prepare_layout_tree_build(); },
        .renew_paint_state = [](void* context) {
            auto& document = *static_cast<Document*>(context);
            document.m_paint_state = make<Painting::DocumentPaintState>(document.layout_node_arena()); },
        .rebuild_list_owners_with_stale_item_counters = [](void* context, u32 const* list_owners, size_t count) {
            auto& document = *static_cast<Document*>(context);
            for (auto list_owner : ReadonlySpan<u32> { list_owners, count }) {
                // An owner that has left the document since the frame named it renders nothing.
                if (auto node = NodeIdentity::of_style_node(CSS::StyleNodeID { list_owner }).resolve(document))
                    node->set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::ListItemCounters);
            } },
        .read_selection = [](void* context, void* sink, void (*receive)(void*, Layout::RustFFI::FfiSelectionSnapshot const*)) {
            auto& document = *static_cast<Document*>(context);
            auto selection = document.get_selection();
            auto range = selection ? selection->range() : nullptr;
            if (!range)
                return;
            Vector<Layout::RustFFI::FfiSelectionSnapshotNode> nodes;
            auto snapshot = Painting::read_selection_snapshot(*range, nodes);
            receive(sink, &snapshot); },
        .apply_layout_commit_effects = [](void* context, Layout::RustFFI::FfiLayoutCommitEffects const* effects) { static_cast<Document*>(context)->apply_layout_commit_effects(*effects); },
        .note_full_layouts_performed = [](void* context, u64 count) { static_cast<Document*>(context)->style_invalidation_counters().relayouts_performed += count; },
        .record_stabilization_bound_failure = [](void* context) { ++static_cast<Document*>(context)->m_style_invalidation_counters.style_stabilization_bound_failures; },
        .attach_style_resources = [](void* context, Compositing::RustFFI::NodeSlotId slot, bool owns_content_replacement_image) {
            auto& document = *static_cast<Document*>(context);
            if (Layout::attach_owed_style_resources(document, slot, owns_content_replacement_image))
                document.m_owed_image_provider_arrived_with_image = true; },
        .attach_generated_image = [](void* context, Compositing::RustFFI::NodeSlotId slot, u32 style_node, Layout::RustFFI::FfiPseudoElement pseudo_element, Layout::RustFFI::FfiGeneratedContentItem item, Compositing::RustFFI::NodeSlotId pseudo_element_box) {
            auto& document = *static_cast<Document*>(context);
            if (Layout::attach_owed_generated_image(document, slot, style_node, pseudo_element, item, pseudo_element_box))
                document.m_owed_image_provider_arrived_with_image = true; },
        .finish_update_layout = [](void* context, Layout::RustFFI::FfiLayoutUpdateEnd end) {
            auto& document = *static_cast<Document*>(context);
            document.style_computer().end_style_record_view_epoch();
            document.end_style_stabilization_epoch();
            Layout::RustFFI::layout_arena_end_update_layout(document.layout_node_arena().handle());
            document.release_held_invalidation_marks();
            document.style_computer().style_engine().publish_inputs_waiting_for_layout_pass();

            // A frame taken back in the middle of main-thread code tells the document nothing that can run script there:
            // its messages and the resnap wait for the next layout update to end, which runs before anything reads them.
            if (end == Layout::RustFFI::FfiLayoutUpdateEnd::FrameTakenBack)
                return;

            // Whatever the pass told the document takes effect before the read that joined for it. That
            // includes the web font faces it reached while they wait on their load.
            document.apply_commit_messages();

            if (document.m_needs_scroll_container_resnap) {
                if (auto navigable = document.navigable(); navigable && navigable->active_document().ptr() == &document)
                    navigable->re_snap_scroll_containers_after_layout_change();
            }

            document.page().client().flush_pending_dom_mutations(); },
    };
}

void Document::update_layout(UpdateLayoutReason reason)
{
    update_layout(reason, ThrottledAnimationSamplingScope::Document);
}

void Document::update_layout(UpdateLayoutReason reason, ThrottledAnimationSamplingScope animation_sampling_scope)
{
    JoinScope join_scope { *this, reason };
    join_scope.update_style_beside_recording(animation_sampling_scope);

    // An image box that owns its image's provider is handed it once the frame that built the box is over, and the
    // frame lays it out without an image. If the image was already there, the box lays out again with it before the
    // read goes on. Only a pass that builds another such box can leave one behind again, so this settles.
    auto update_style_and_layout = [&] {
        update_style_and_layout_once(reason, animation_sampling_scope);
        while (exchange(m_owed_image_provider_arrived_with_image, false)) {
            join_scope.note_extra_pass();
            update_style_and_layout_once(reason, animation_sampling_scope);
        }
    };

    update_style_and_layout();

    // AD-HOC: A scroll-state() query against a container that has not been snapshotted yet reads no state. Like other
    //         engines, take such a container's first snapshot as soon as its layout is known, so that the style it
    //         decides is right before the next rendering update. Later changes of its state wait for that update.
    while (layout_is_up_to_date() && m_scroll_state_query_containers.snapshot_post_layout_state(*this, CSS::ScrollStateQueryContainers::Snapshot::NewContainersOnly)) {
        join_scope.note_extra_pass();
        update_style_and_layout();
    }
}

bool Document::submit_layout_for_rendering_update()
{
    return update_style_and_layout_once(UpdateLayoutReason::HTMLEventLoopRenderingUpdate, ThrottledAnimationSamplingScope::Document, LayoutPassSubmission::MaySubmit);
}

bool Document::update_style_and_layout_once(UpdateLayoutReason reason, ThrottledAnimationSamplingScope animation_sampling_scope, LayoutPassSubmission pass_submission)
{
    auto navigable = this->navigable();
    if (!navigable || navigable->active_document().ptr() != this)
        return false;

    // Internal layout dependencies do not observe compositor animation values.
    if (reason != UpdateLayoutReason::HTMLEventLoopRenderingUpdate
        && reason != UpdateLayoutReason::ChildDocumentStyleUpdate
        && animation_sampling_scope == ThrottledAnimationSamplingScope::Document)
        flush_throttled_animation_style_update();

    // A read made beside this document's frame in flight waits for it, and then runs a frame of its
    // own for what changed meanwhile.
    join_frame_in_flight();

    // The style update in this document's frame lays out its container documents first, from work the frame joins the
    // document thread for, and such work cannot take in a frame in flight. So a frame in flight that holds one of them
    // is taken in before this document's frame starts.
    for (auto container = container_document(); container; container = container->container_document())
        container->join_frame_in_flight();

    // Every mark the DOM side has made goes through before the pass that reads them starts; marks
    // made from inside the pass write through on their own.
    drain_invalidation_journal();

    auto& arena = layout_node_arena();
    Layout::RustFFI::layout_arena_begin_update_layout(arena.handle());

    begin_style_stabilization_epoch();

    // Keep shared style records alive across both style and layout, so temporary views
    // during layout tree construction and layout do not need individual record pins.
    style_computer().begin_style_record_view_epoch();

    // NB: The update, and the epochs begun above, end as the frame's end is taken in (finish_update_layout in the host
    //     callbacks): before layout_arena_update_layout returns, or once a submitted pass's frame is taken back.

    Layout::RustFFI::FfiLayoutUpdateInputs inputs {
        .reason_is_inspect_devtools_layout_data = reason == UpdateLayoutReason::InspectDevToolsLayoutData,
        .is_template_contents_document = m_created_for_appropriate_template_contents,
        .reason_name = ffi_utf16_view(to_string(reason)),
        .may_submit_pass = pass_submission == LayoutPassSubmission::MaySubmit,
    };
    return Layout::RustFFI::layout_arena_update_layout(arena.handle(), &inputs) == Layout::RustFFI::FfiLayoutUpdateOutcome::PassSubmitted;
}

}
