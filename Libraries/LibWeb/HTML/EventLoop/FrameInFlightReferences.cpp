/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGC/Cell.h>
#include <LibGC/CellAllocator.h>
#include <LibGC/Heap.h>
#include <LibGC/Root.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/FrameInFlightReferences.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/Page/Page.h>

namespace Web::HTML {

namespace {

// The documents a frame in flight holds. It keeps them alive itself, since it ends their style record view epochs.
class FrameInFlightHolds final : public GC::Cell {
    GC_CELL(FrameInFlightHolds, GC::Cell);
    GC_DECLARE_ALLOCATOR(FrameInFlightHolds);

public:
    void hold(DOM::Document& document)
    {
        if (m_documents.contains_slow(GC::Ref { document }))
            return;
        m_documents.append(document);
        // NB: This is the style engine's epoch, not StyleComputer's: the frame only holds off reclamation, and views
        //     made on the main thread meanwhile still pin their records as they would outside a frame.
        document.style_computer().style_engine().begin_style_record_view_epoch();
    }

    void release()
    {
        for (auto const& document : m_documents)
            document->style_computer().style_engine().end_style_record_view_epoch();
        m_documents.clear();
    }

    bool holds(DOM::Document const& document) const
    {
        return any_of(m_documents, [&](auto const& entry) { return entry.ptr() == &document; });
    }

    bool references_are_alive() const
    {
        auto is_alive = [](GC::Cell const& cell) { return cell.state() == GC::Cell::State::Live; };
        return all_of(m_documents, [&](auto const& document) {
            if (!is_alive(*document) || !is_alive(document->page()))
                return false;
            if (auto navigable = document->navigable(); navigable && !is_alive(*navigable))
                return false;
            if (auto window = document->window(); window && !is_alive(*window))
                return false;
            return true;
        });
    }

private:
    virtual void visit_edges(Visitor& visitor) override
    {
        Base::visit_edges(visitor);
        visitor.visit(m_documents);
    }

    Vector<GC::Ref<DOM::Document>> m_documents;
};

GC_DEFINE_ALLOCATOR(FrameInFlightHolds);

}

// Allocated when the first document's frame goes to the render side and freed once the frame is consumed, so that the
// root never outlives the heap.
static GC::Root<FrameInFlightHolds>* s_frame_in_flight_holds;

void hold_for_frame_in_flight(DOM::Document& document)
{
    if (!s_frame_in_flight_holds)
        s_frame_in_flight_holds = new GC::Root<FrameInFlightHolds> { GC::Heap::the().allocate<FrameInFlightHolds>() };
    (*s_frame_in_flight_holds)->hold(document);
}

void release_holds_for_frame_in_flight()
{
    if (!s_frame_in_flight_holds)
        return;
    (*s_frame_in_flight_holds)->release();
    delete s_frame_in_flight_holds;
    s_frame_in_flight_holds = nullptr;
}

bool frame_in_flight_holds(DOM::Document const& document)
{
    return s_frame_in_flight_holds && (*s_frame_in_flight_holds)->holds(document);
}

bool frame_in_flight_references_are_alive()
{
    return !s_frame_in_flight_holds || (*s_frame_in_flight_holds)->references_are_alive();
}

}
