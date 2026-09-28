/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/HTML/EventLoop/MainThreadPhases.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Selection/Selection.h>

namespace Web::DOM {

// Reads the document's selection range, when it has one, into `snapshot`, whose nodes `nodes` holds. Answers the
// snapshot, or null when the document has no selection range.
static Layout::RustFFI::FfiSelectionSnapshot const* read_selection(Document& document, Layout::RustFFI::FfiSelectionSnapshot& snapshot, Vector<Layout::RustFFI::FfiSelectionSnapshotNode>& nodes)
{
    auto selection = document.get_selection();
    auto range = selection ? selection->range() : nullptr;
    if (!range)
        return nullptr;
    snapshot = Painting::read_selection_snapshot(*range, nodes);
    return &snapshot;
}

// What the document reads for a layout round once the round's style has run, with the storage the selection it read
// lives in.
struct LayoutRoundReading {
    Layout::RustFFI::FfiSelectionSnapshot selection {};
    Vector<Layout::RustFFI::FfiSelectionSnapshotNode> selection_nodes;
    Layout::RustFFI::FfiLayoutRoundFacts round {};
};

// Whether the tree build of the next round may build the viewport: the document has none yet, the whole tree is to be
// built again, or the document node itself is. A document hosting template contents builds no viewport.
static bool layout_tree_build_may_create_viewport(Document& document)
{
    if (document.created_for_appropriate_template_contents())
        return false;
    return !document.has_layout_root()
        || Layout::RustFFI::layout_arena_needs_full_layout_tree_update(Layout::document_layout_arena(document))
        || document.needs_layout_tree_update();
}

// A round goes on from the facts and the selection the document reads once the list item renumbers and top layer
// changes its style leaves have gone through (`read_facts` takes those in, and reads the facts). A round whose tree
// build may build the viewport is handed the document's style with them, which the document makes on demand rather
// than publishing: the owner runs the round without asking for it.
// Only the commits of a round that lays out read the selection, to stamp the boxes they build, so a round whose layout
// is up to date (and that no DevTools client may force to lay out) is not handed one: reading it walks every node in its
// range.
template<typename ReadFacts>
static void read_layout_round(Document& document, LayoutRoundReading& reading, ReadFacts&& read_facts)
{
    auto facts = read_facts();
    bool may_lay_out = facts.should_collect_devtools_layout_data || !document.layout_is_up_to_date();
    reading.selection_nodes.clear();
    reading.round = {
        .facts = facts,
        .selection = may_lay_out ? read_selection(document, reading.selection, reading.selection_nodes) : nullptr,
        .has_document_style = false,
        .document_style = {},
    };
    if (layout_tree_build_may_create_viewport(document)) {
        reading.round.has_document_style = true;
        reading.round.document_style = Layout::document_style_for_build(document);
    }
}

// The document-side steps of the layout update, which the Rust side runs through this table between the frame jobs it
// sends the render owner.
Layout::RustFFI::FfiLayoutUpdateHostCallbacks Document::layout_update_host_callbacks()
{
    return {
        .context = this,
        .document_facts = [](void* context) { return static_cast<Document*>(context)->layout_update_document_facts(); },
        .take_in_frame_effects = [](void* context, Layout::RustFFI::FfiLayoutFrameEffects const* effects) { static_cast<Document*>(context)->take_in_layout_frame_effects(*effects); },
        .start_round = [](void* context, bool runs_style, void* sink) {
            auto& document = *static_cast<Document*>(context);
            if (runs_style)
                document.update_style();
            LayoutRoundReading reading;
            read_layout_round(document, reading, [&] {
                document.process_pending_list_item_renumbers();
                document.process_pending_top_layer_layout_changes();
                return document.layout_update_document_facts();
            });
            Layout::RustFFI::layout_frame_take_round(sink, &reading.round); },
    };
}

// What a layout round reads from the document once the round's style has run.
Layout::RustFFI::FfiLayoutUpdateDocumentFacts Document::layout_update_document_facts()
{
    auto navigable = this->navigable();
    bool document_is_active = navigable && navigable->active_document().ptr() == this;
    auto viewport_rect = document_is_active ? navigable->viewport_rect() : CSSPixelRect {};
    return {
        .document_is_active = document_is_active,
        .document_needs_layout_tree_build = needs_layout_tree_update() || child_needs_layout_tree_update(),
        .style_input_waits_on_document = render_inputs().needs_animated_style_update()
            || style_computer().style_engine().has_recorded_input()
            || style_computer().style_engine().has_install_feedback()
            || render_inputs().needs_media_rule_evaluation()
            || render_inputs().has_elements_with_dirty_style_attributes(),
        .top_layer_work_pending = render_inputs().has_pending_top_layer_change(),
        .should_collect_devtools_layout_data = page().client().has_active_devtools_client(),
        .document_in_quirks_mode = in_quirks_mode(),
        .viewport_inline_size_raw = viewport_rect.width().raw_value(),
        .viewport_block_size_raw = viewport_rect.height().raw_value(),
        .document_style_node = style_node_id().value(),
    };
}

void Document::renew_clock_layout_frame()
{
    auto* arena = Layout::document_layout_arena_if_created(*this);
    if (!arena)
        return;
    // The clock's ticks lay the document out from these facts beside the main thread, so its geometry moves with no
    // write of the main thread's to show for it.
    (void)render_inputs_for_write();
    Layout::RustFFI::FfiSelectionSnapshot selection {};
    Vector<Layout::RustFFI::FfiSelectionSnapshotNode> selection_nodes;
    Layout::RustFFI::FfiLayoutRoundFacts round {
        .facts = layout_update_document_facts(),
        .selection = read_selection(*this, selection, selection_nodes),
        .has_document_style = false,
        .document_style = {},
    };
    Layout::RustFFI::layout_arena_renew_clock_layout_frame(arena, &round);
}

// Ends the layout update a layout frame ran on the document side, once the frame is over: the epochs the update began,
// the arena's update, and the marks held beside the frame.
void Document::end_layout_frame_update(void* arena)
{
    style_computer().end_style_record_view_epoch();
    end_style_stabilization_epoch();
    Layout::RustFFI::layout_arena_end_update_layout(arena);
    release_held_invalidation_marks();
    render_inputs_for_write().style_engine().publish_inputs_waiting_for_layout_pass();
}

// Ends a layout update on the document side, whether or not it ran a frame: whatever the frames before it told the
// document takes effect before the read that asked for the update.
static void end_layout_update_on_document_side(Document& document)
{
    // Whatever the pass told the document takes effect before the read that joined for it. That includes the web font
    // faces it reached while they wait on their load.
    document.apply_commit_messages();

    if (document.needs_scroll_container_resnap()) {
        if (auto navigable = document.navigable(); navigable && navigable->active_document().ptr() == &document)
            navigable->re_snap_scroll_containers_after_layout_change();
    }

    document.page().client().flush_pending_dom_mutations();
}

// Takes in what a layout frame left for the document once it is over, in the order the frame leaves it, and ends the
// layout update on the document side.
void Document::take_in_layout_frame_effects(Layout::RustFFI::FfiLayoutFrameEffects const& effects)
{
    auto* arena = Layout::document_layout_arena(*this);

    // A document retiring its render state takes the frame back only to end the update: the rows the frame owes
    // resources, the commit it made and the rendering it prepared go away with the render state, and nothing of it is
    // published. So what the frame leaves for the host is dropped, and the document is told nothing.
    if (m_retiring_render_state) {
        end_layout_frame_update(arena);
        return;
    }

    // An image box that owns its image's provider is handed it here, and lays out again if the image is already there.
    for (auto const& owed : ReadonlySpan<Layout::RustFFI::FfiOwedImageResources> { effects.owed_image_resources, effects.owed_image_resources_count }) {
        bool image_was_available = false;
        switch (owed.tag) {
        case Layout::RustFFI::FfiOwedImageResources::Tag::StyleResources: {
            auto const& resources = owed.style_resources;
            Layout::RustFFI::layout_arena_hand_over_owed_image_resources(arena, resources.row);
            image_was_available = Layout::attach_owed_style_resources(*this, resources.row, resources.owns_content_replacement_image);
            break;
        }
        case Layout::RustFFI::FfiOwedImageResources::Tag::GeneratedImage: {
            auto const& image = owed.generated_image;
            Layout::RustFFI::layout_arena_hand_over_owed_image_resources(arena, image.row);
            image_was_available = Layout::attach_owed_generated_image(*this, image.row, image.generator, image.pseudo_element, image.item, image.pseudo_element_box);
            break;
        }
        }
        if (image_was_available)
            m_owed_image_provider_arrived_with_image = true;
    }

    m_style_invalidation_counters.relayouts_performed += effects.full_layouts_performed;

    if (effects.commit.layout_committed)
        apply_layout_commit_effects(effects.commit);

    if (effects.prepare_for_rendering)
        prepare_for_rendering();

    end_layout_frame_update(arena);

    // A frame taken back in the middle of main-thread code tells the document nothing that can run script there: its
    // messages and the resnap wait for the next layout update to end, which runs before anything reads them.
    if (effects.end == Layout::RustFFI::FfiLayoutUpdateEnd::FrameTakenBack)
        return;

    end_layout_update_on_document_side(*this);
}

void Document::update_layout(UpdateLayoutReason reason)
{
    update_layout(reason, ThrottledAnimationSamplingScope::Document);
}

void Document::update_layout(UpdateLayoutReason reason, ThrottledAnimationSamplingScope animation_sampling_scope)
{
    // Nothing was written to the render inputs of the document or of any document embedding it since they published
    // their query snapshots: style and layout are what the snapshots say, and a frame would lay nothing out. Only a
    // DevTools inspection lays out a clean document, to collect its layout data. What a frame taken back before the
    // snapshot left for the document side is still taken in.
    if (reason != UpdateLayoutReason::InspectDevToolsLayoutData
        && query_view_for_clean_read().has_value()
        && !m_commit_messages->has_queued_navigable_container_viewport()) {
        end_layout_update_on_document_side(*this);
        return;
    }

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

    auto* arena = Layout::document_layout_arena(*this);
    Layout::RustFFI::layout_arena_begin_update_layout(arena);

    begin_style_stabilization_epoch();

    // Keep shared style records alive across both style and layout, so temporary views
    // during layout tree construction and layout do not need individual record pins.
    style_computer().begin_style_record_view_epoch();

    // NB: The update, and the epochs begun above, end as the frame's end is taken in (take_in_layout_frame_effects):
    //     before layout_arena_update_layout returns, or once a submitted pass's frame is taken back.

    bool const may_submit_pass = pass_submission != LayoutPassSubmission::Wait;
    // The update's first round's style runs here, ahead of the update.
    update_style();

    LayoutRoundReading first_round;
    read_layout_round(*this, first_round, [&] {
        process_pending_list_item_renumbers();
        process_pending_top_layer_layout_changes();
        return layout_update_document_facts();
    });

    Layout::RustFFI::FfiLayoutUpdateInputs inputs {
        .reason_is_inspect_devtools_layout_data = reason == UpdateLayoutReason::InspectDevToolsLayoutData,
        .is_template_contents_document = m_created_for_appropriate_template_contents,
        .may_submit_pass = may_submit_pass,
        .first_round = first_round.round,
    };
    auto outcome = Layout::RustFFI::layout_arena_update_layout(arena, &inputs);
    if (outcome == Layout::RustFFI::FfiLayoutUpdateOutcome::FlightReady) {
        // The flight records the document after its layout only if the document seals what that reads before it submits
        // the flight. A document that is to update its style after the layout lays out again before it shows anything.
        if (auto navigable = this->navigable())
            navigable->seal_flight_paint(*this, !needs_style_update_after_layout());
        Layout::RustFFI::layout_arena_submit_prepared_flight(arena);
        return true;
    }
    return outcome == Layout::RustFFI::FfiLayoutUpdateOutcome::PassSubmitted;
}

}
