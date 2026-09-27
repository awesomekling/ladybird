/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/HTML/EventLoop/MainThreadPhases.h>
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
                .style_input_waits_on_document = document.m_needs_animated_style_update
                    || document.style_computer().style_engine().has_recorded_input()
                    || document.m_needs_media_rule_evaluation
                    || !document.m_elements_with_dirty_style_attributes.is_empty(),
                .top_layer_work_pending = document.m_top_layer_needs_layout_zone_rebuild || !document.m_elements_with_pending_top_layer_membership_change.is_empty(),
                .should_collect_devtools_layout_data = document.page().client().has_active_devtools_client(),
                .document_in_quirks_mode = document.in_quirks_mode(),
                .viewport_inline_size_raw = viewport_rect.width().raw_value(),
                .viewport_block_size_raw = viewport_rect.height().raw_value(),
            }; },
        .prepare_for_rendering = [](void* context) { static_cast<Document*>(context)->prepare_for_rendering(); },
        .seal_flight_paint = [](void* context, bool style_runs_in_flight) {
            auto& document = *static_cast<Document*>(context);
            // A document that is to update its style after the layout lays out again before it shows anything. The style
            // a flight runs ahead of its layout is not such an update: the flight's take-back installs it.
            if (auto navigable = document.navigable())
                navigable->seal_flight_paint(document, !document.needs_style_update_after_layout(style_runs_in_flight)); },
        .prepare_layout_tree_build = [](void* context) -> u32 { return static_cast<Document*>(context)->prepare_layout_tree_build(); },
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
        .finish_submitted_style_update = [](void* context) { static_cast<Document*>(context)->finish_style_update_submitted_in_flight(); },
        .settle_flight_style_repaint = [](void* context, bool recorded_in_flight) { static_cast<Document*>(context)->settle_style_repaint_owed_to_flight(recorded_in_flight); },
    };
}

void Document::update_layout(UpdateLayoutReason reason)
{
    update_layout(reason, ThrottledAnimationSamplingScope::Document);
}

void Document::update_layout(UpdateLayoutReason reason, ThrottledAnimationSamplingScope animation_sampling_scope)
{
    HTML::MainThreadPhases::Scope phase { HTML::MainThreadPhases::layout_phase(*this) };
    JoinScope join_scope { *this, reason };

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

// A style update whose first pass leaves the layout tree as it is runs in the flight that lays the document out after it:
// the layout update begins as the rendering update's would, and its first round submits the style pass for the flight to
// run instead of running style itself, unless the document's layout tree is to be built again first.
// Opt-in for now (LIBWEB_FLIGHT_STYLE=1): otherwise the style pass runs in a flight of its own ahead of the layout.
bool Document::style_runs_in_layout_flights()
{
    static bool const enabled = [] {
        auto const* value = getenv("LIBWEB_FLIGHT_STYLE");
        return value && StringView { value, strlen(value) } == "1"sv;
    }();
    return enabled;
}

bool Document::submit_style_and_layout_for_rendering_update()
{
    if (!style_runs_in_layout_flights() || m_created_for_appropriate_template_contents || !has_layout_root())
        return false;
    return update_style_and_layout_once(UpdateLayoutReason::HTMLEventLoopRenderingUpdate, ThrottledAnimationSamplingScope::Document, LayoutPassSubmission::MaySubmitWithStyle);
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
    // is taken in before this document's frame starts. A style update the main thread waits for lays them out only if
    // one of them has style or layout work to do; a submitted one may find work that a task beside it leaves them.
    if (pass_submission != LayoutPassSubmission::Wait || !CSS::embedding_document_chain_has_no_pending_style_or_layout_work(*this)) {
        for (auto container = container_document(); container; container = container->container_document())
            container->join_frame_in_flight();
    }

    // Every mark the DOM side has made goes through before the pass that reads them starts; marks
    // made from inside the pass write through on their own.
    drain_invalidation_journal();

    // The style a flight runs ahead of its layout is the style of a layout update that builds no layout tree first and
    // has no host step between its style and its layout that reads style: otherwise the style runs in a flight of its
    // own, and the layout on the main thread after it.
    if (pass_submission == LayoutPassSubmission::MaySubmitWithStyle
        && (needs_layout_tree_update() || child_needs_layout_tree_update() || m_top_layer_needs_layout_zone_rebuild
            || !m_elements_with_pending_top_layer_membership_change.is_empty() || !m_list_owners_pending_item_renumber.is_empty()
            || Layout::RustFFI::layout_arena_needs_full_layout_tree_update(layout_node_arena().handle())))
        return false;

    auto& arena = layout_node_arena();
    Layout::RustFFI::layout_arena_begin_update_layout(arena.handle());

    begin_style_stabilization_epoch();

    // Keep shared style records alive across both style and layout, so temporary views
    // during layout tree construction and layout do not need individual record pins.
    style_computer().begin_style_record_view_epoch();

    // NB: The update, and the epochs begun above, end as the frame's end is taken in (finish_update_layout in the host
    //     callbacks): before layout_arena_update_layout returns, or once a submitted pass's frame is taken back.

    bool const may_submit_pass = pass_submission != LayoutPassSubmission::Wait;
    // The update's first round's style runs here, ahead of the update. One that runs in the flight begins here: its pass
    // is submitted for the update to collect, and the rest of the style update is installed as the flight is taken back.
    // The style of the rounds after the first runs as each of them starts.
    bool const style_in_flight = pass_submission == LayoutPassSubmission::MaySubmitWithStyle
        && Layout::RustFFI::layout_arena_collect_style_pass_for_flight(arena.handle(), may_submit_pass);
    if (style_in_flight)
        submit_style_for_flight();
    else
        update_style();

    Layout::RustFFI::FfiLayoutUpdateInputs inputs {
        .reason_is_inspect_devtools_layout_data = reason == UpdateLayoutReason::InspectDevToolsLayoutData,
        .is_template_contents_document = m_created_for_appropriate_template_contents,
        .reason_name = ffi_utf16_view(to_string(reason)),
        .may_submit_pass = may_submit_pass,
        .style_in_flight = style_in_flight,
        .viewport_propagation_sources = {},
    };
    if (style_in_flight) {
        auto sources = CSS::StyleEffectDrain::viewport_propagation_sources_of(*this);
        for (size_t index = 0; index < sources.size() && index < 2; ++index)
            inputs.viewport_propagation_sources[index] = sources[index].value();
    }
    return Layout::RustFFI::layout_arena_update_layout(arena.handle(), &inputs) == Layout::RustFFI::FfiLayoutUpdateOutcome::PassSubmitted;
}

}
