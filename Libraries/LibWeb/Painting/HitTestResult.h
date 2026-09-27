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
#include <AK/Types.h>
#include <LibGC/Ptr.h>
#include <LibWeb/DOM/AbstractRange.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/ChromeWidget.h>
#include <LibWeb/TextAffinity.h>
#include <LibWebCommon/PixelUnits.h>

namespace Web::Painting {

// A document's hit-test snapshot (hit_test/snapshot.rs), as a hit test and the hits it found hold it. It is immutable
// and owns everything it holds, and it holds nothing of the layout node arena.
class WEB_API HitTestSnapshot : public RefCounted<HitTestSnapshot> {
public:
    static NonnullRefPtr<HitTestSnapshot const> adopt(void const* handle);
    ~HitTestSnapshot();

    void const* handle() const { return m_handle; }

private:
    explicit HitTestSnapshot(void const* handle)
        : m_handle(handle)
    {
    }

    void const* m_handle { nullptr };
};

// A box of the hit-test snapshot a hit was found in, as the snapshot was published: what an event dispatched through
// the hit reads of the boxes it went through. It is not a layout node or an arena slot, and only the snapshot's reads
// take it.
class WEB_API HitBox {
public:
    static Optional<HitBox> of(NonnullRefPtr<HitTestSnapshot const>, Layout::RustFFI::FfiHitBox);

    bool has_committed_box() const { return m_facts.has_committed_box; }
    bool is_navigable_container_viewport() const { return has_committed_box() && m_facts.kind == Layout::RustFFI::NodeKind::NavigableContainerViewport; }
    Optional<CSS::PseudoElement> generated_for_pseudo_element() const;
    // The DOM node the box stands for: nothing for an anonymous box.
    DOM::NodeIdentity identity() const;
    // The element a box generated for a pseudo-element was generated for.
    DOM::NodeIdentity generator_identity() const;
    Layout::RustFFI::FfiCrossProcessId local_content_navigable() const { return m_facts.local_content_navigable; }
    CSSPixelRect absolute_rect() const { return m_facts.absolute_rect; }
    CSSPixelPoint box_type_agnostic_position() const { return m_facts.box_type_agnostic_position; }

    // The box the DOM node was bound to when the snapshot was published.
    static Optional<HitBox> bound_box_in(NonnullRefPtr<HitTestSnapshot const>, DOM::NodeIdentity);

    Optional<HitBox> parent() const;
    // The box the DOM node was bound to in the same snapshot.
    Optional<HitBox> bound_box_of(DOM::NodeIdentity) const;

    // As transform_to_local_coordinates() and inverse_transform_point() convert a point for a layout node's box.
    CSSPixelPoint transform_to_local_coordinates(DOM::Document const&, CSSPixelPoint) const;
    CSSPixelPoint inverse_transform_point(DOM::Document const&, CSSPixelPoint) const;

private:
    HitBox(NonnullRefPtr<HitTestSnapshot const>, Layout::RustFFI::FfiHitBox);

    NonnullRefPtr<HitTestSnapshot const> m_snapshot;
    Layout::RustFFI::FfiHitBoxFacts m_facts;
};

WEB_API DOM::NodeIdentity identity_of_hit_node(Layout::RustFFI::FfiHitNodeIdentity);

struct WEB_API HitTestResult {
    DOM::NodeIdentity node;
    // The box whose style admitted the hit, if the snapshot names one.
    Optional<HitBox> box;
    // The row of that box, which only scrolling still takes a layout node of.
    Compositing::RustFFI::NodeSlotId hit_node;
    NonnullRefPtr<Layout::NodeArena> arena;
    RefPtr<ChromeWidget> chrome_widget {};
    size_t index_in_node { 0 };
    bool is_text_fragment { false };

    DOM::Node* dom_node() const;
    // The layout node a scroll the hit starts walks the containing block chain up from.
    Layout::Node* scrolling_layout_node() const { return layout_node_for_committed_slot(*arena, hit_node); }
};

// A boundary point that names its node instead of pointing at it. A node that left the tree since
// the hit test resolves to nothing, where a pointer would have handed back a node the document no
// longer contains.
struct WEB_API BoundaryIdentity {
    DOM::NodeIdentity node;
    WebIDL::UnsignedLong offset { 0 };

    Optional<DOM::BoundaryPoint> resolve(DOM::Document&) const;
};

struct WEB_API CaretPosition {
    Compositing::RustFFI::NodeSlotId paintable;
    NonnullRefPtr<Layout::NodeArena> arena;
    BoundaryIdentity boundary;
    TextAffinity affinity { TextAffinity::Downstream };
    Optional<BoundaryIdentity> secondary_boundary {};
    Optional<CSSPixelRect> debug_rect {};

    GC::Ptr<DOM::Node> boundary_node() const;
    Optional<DOM::BoundaryPoint> boundary_point() const;
    // The row the boundary's node is bound to, found in the arena rather than asked of that node.
    Layout::Node* boundary_layout_node() const;
    Layout::Node* layout_node() const { return layout_node_for_committed_slot(*arena, paintable); }
};

}
