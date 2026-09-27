/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullRefPtr.h>
#include <AK/Optional.h>
#include <AK/RefCounted.h>
#include <AK/RefPtr.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/PixelUnits.h>

namespace Web::Painting {

// Whether a query snapshot is published with the visual contexts it converts every box's rects to viewport space
// through, or only converts the rects of a box that no transform, sticky offset or scroll offset moves.
enum class QueryVisualContexts : u8 {
    UpToDate,
    Stale,
};

// A document's committed geometry, as the document published it for the main thread's geometry reads (query_snapshot.rs).
// It is immutable and owns everything it holds, and it holds nothing of the layout node arena. The document holds it
// until something is written to its render inputs (DOM::RenderInputs): while it does, the snapshot describes it.
class QuerySnapshot : public RefCounted<QuerySnapshot> {
public:
    // Null while a stage runs: the arena then has no committed geometry to publish.
    static RefPtr<QuerySnapshot const> publish(DOM::Document const&, QueryVisualContexts);
    ~QuerySnapshot();

    QueryVisualContexts visual_contexts() const { return m_visual_contexts; }

private:
    friend class QueryView;

    QuerySnapshot(void const* handle, QueryVisualContexts);

    void const* m_handle { nullptr };
    QueryVisualContexts m_visual_contexts { QueryVisualContexts::Stale };
};

// A box of a query snapshot. Only the snapshot's view takes it: it is not a layout node or an arena slot.
struct QueryBox {
    Layout::RustFFI::FfiQueryBox ffi;
};

// What an offset read asks of a box.
struct QueryBoxFacts {
    bool has_committed_box { false };
    CSS::Positioning position { CSS::Positioning::Static };
    // Positioned as painting counts it: a position other than static, or a z-index on a flex or grid item.
    bool is_positioned_for_painting { false };
    bool establishes_an_absolute_positioning_containing_block { false };
    bool establishes_a_fixed_positioning_containing_block { false };

    bool is_positioned() const { return position != CSS::Positioning::Static; }
    bool is_fixed_position() const { return position == CSS::Positioning::Fixed; }
};

// What a clean geometry read reads: one query snapshot, and nothing else. It holds no arena and no layout node, so an
// answer computed through it cannot reach render-side state.
class QueryView {
public:
    explicit QueryView(NonnullRefPtr<QuerySnapshot const> snapshot)
        : m_snapshot(move(snapshot))
    {
    }

    // The box the element is bound to: its layout node.
    Optional<QueryBox> box_of(DOM::Element const&) const;
    // The element's principal box: the table wrapper box of a table.
    Optional<QueryBox> principal_box_of(DOM::Element const&) const;

    QueryBoxFacts facts(QueryBox) const;
    bool any_ancestor_establishes_a_fixed_position_containing_block(QueryBox) const;
    CSSPixelRect absolute_border_box_rect(QueryBox) const;
    CSSPixelRect absolute_padding_box_rect(QueryBox) const;

    // Empty if the snapshot cannot convert the box's rects to viewport space.
    Optional<Vector<CSSPixelRect>> client_rects(QueryBox) const;
    Optional<CSSPixelRect> bounding_client_rect(QueryBox) const;

private:
    NonnullRefPtr<QuerySnapshot const> m_snapshot;
};

}
