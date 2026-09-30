/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/PaintFacts.h>
#include <LibWeb/Painting/StyleImageObservers.h>

namespace Web::Painting {

StyleImageObserver::StyleImageObserver(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, NonnullRefPtr<CSS::ImageStyleValue const> image)
    : CSS::ImageStyleValue::Client(document, *image)
    , m_slot(slot)
    , m_image(move(image))
{
}

StyleImageObserver::~StyleImageObserver()
{
    image_style_value_finalize();
}

static void push_layer_image_paint_facts_and_repaint(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
{
    auto box = BoxSlot::of(document, slot);
    if (!box)
        return;
    push_layer_image_paint_facts(box);
    if (has_committed_box(box))
        set_needs_repaint(box, InvalidateDisplayList::PaintCommands);
}

void StyleImageObserver::image_style_value_did_update(CSS::ImageStyleValue&)
{
    auto document = this->document();
    if (!document)
        return;

    // Beside a frame that owns the arena, the row's facts are pushed once the frame has been taken in. By then its row
    // may be gone, and its slot may hold another row: the facts are pushed only while this observer, which goes with
    // its row, is still alive.
    if (HTML::FrameScheduler::arena_changes_wait_for_frame(*document)) {
        HTML::main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(document->heap(), [observer = make_weak_ptr(), document = GC::Weak<DOM::Document> { *document }, slot = m_slot] {
            if (observer && document)
                push_layer_image_paint_facts_and_repaint(*document, slot);
        }));
        return;
    }
    push_layer_image_paint_facts_and_repaint(*document, m_slot);
}

static StyleImageObserver const* observer_at(Vector<OwnPtr<StyleImageObserver>> const& observers, size_t index)
{
    if (index >= observers.size())
        return nullptr;
    return observers[index].ptr();
}

StyleImageObserver const* StyleImageObserverSet::background_image_observer(size_t layer_index) const
{
    return observer_at(background_layers, layer_index);
}

StyleImageObserver const* StyleImageObserverSet::mask_image_observer(size_t layer_index) const
{
    return observer_at(mask_layers, layer_index);
}

StyleImageObserver const* StyleImageObserverSet::cursor_image_observer(size_t cursor_index) const
{
    return observer_at(cursors, cursor_index);
}

StyleImageObserverSet const* style_image_observers(BoxSlot const& box)
{
    if (!box)
        return nullptr;
    return static_cast<StyleImageObserverSet const*>(Layout::RustFFI::layout_arena_image_observers(box.arena(), box.slot()));
}

void replace_style_image_observers(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, OwnPtr<StyleImageObserverSet> observers)
{
    auto* arena = Layout::document_layout_arena_if_created(document);
    if (!arena) {
        VERIFY(!observers);
        return;
    }
    // The new set registers before the old one unregisters, so a shared resource is never dropped and refetched.
    delete static_cast<StyleImageObserverSet*>(Layout::RustFFI::layout_arena_replace_image_observers(arena, slot, observers.leak_ptr()));
}

}

extern "C" WEB_API void ladybird_layout_image_observers_destroy(void* image_observers)
{
    delete static_cast<Web::Painting::StyleImageObserverSet*>(image_observers);
}
