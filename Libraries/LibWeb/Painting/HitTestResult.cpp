/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/HitTestResult.h>

namespace Web::Painting {

NonnullRefPtr<HitTestSnapshot const> HitTestSnapshot::adopt(void const* handle)
{
    VERIFY(handle);
    return adopt_ref(*new HitTestSnapshot(handle));
}

HitTestSnapshot::~HitTestSnapshot()
{
    Layout::RustFFI::hit_test_snapshot_release(m_handle);
}

DOM::NodeIdentity identity_of_hit_node(Layout::RustFFI::FfiHitNodeIdentity identity)
{
    switch (identity.kind) {
    case Layout::RustFFI::FfiHitNodeIdentityKind::None:
        return {};
    case Layout::RustFFI::FfiHitNodeIdentityKind::StyleNode:
        return DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { identity.style_node });
    case Layout::RustFFI::FfiHitNodeIdentityKind::Document:
        return DOM::NodeIdentity::of_document();
    }
    VERIFY_NOT_REACHED();
}

static Layout::RustFFI::FfiHitNodeIdentity ffi_identity_of(DOM::NodeIdentity identity)
{
    if (identity == DOM::NodeIdentity::of_document())
        return { Layout::RustFFI::FfiHitNodeIdentityKind::Document, 0 };
    if (auto style_node = identity.style_node(); style_node != CSS::StyleNodeID {})
        return { Layout::RustFFI::FfiHitNodeIdentityKind::StyleNode, style_node.value() };
    return { Layout::RustFFI::FfiHitNodeIdentityKind::None, 0 };
}

HitBox::HitBox(NonnullRefPtr<HitTestSnapshot const> snapshot, Layout::RustFFI::FfiHitBox box)
    : m_snapshot(move(snapshot))
    , m_facts(Layout::RustFFI::hit_test_snapshot_box_facts(m_snapshot->handle(), box))
{
}

Optional<HitBox> HitBox::of(NonnullRefPtr<HitTestSnapshot const> snapshot, Layout::RustFFI::FfiHitBox box)
{
    if (box.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return {};
    return HitBox { move(snapshot), box };
}

Optional<CSS::PseudoElement> HitBox::generated_for_pseudo_element() const
{
    if (m_facts.generated_for == 0)
        return {};
    return static_cast<CSS::PseudoElement>(m_facts.generated_for - 1);
}

DOM::NodeIdentity HitBox::identity() const
{
    return identity_of_hit_node(m_facts.identity);
}

DOM::NodeIdentity HitBox::generator_identity() const
{
    return identity_of_hit_node(m_facts.generator);
}

Optional<HitBox> HitBox::parent() const
{
    return of(m_snapshot, m_facts.parent);
}

Optional<HitBox> HitBox::bound_box_in(NonnullRefPtr<HitTestSnapshot const> snapshot, DOM::NodeIdentity identity)
{
    auto box = Layout::RustFFI::hit_test_snapshot_bound_box(snapshot->handle(), ffi_identity_of(identity));
    return of(move(snapshot), box);
}

Optional<HitBox> HitBox::bound_box_of(DOM::NodeIdentity identity) const
{
    return bound_box_in(m_snapshot, identity);
}

CSSPixelPoint HitBox::transform_to_local_coordinates(DOM::Document const& document, CSSPixelPoint position) const
{
    if (!has_committed_box())
        return {};
    if (!document.is_rendered())
        return position;
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto result = document.visual_context_tree().transform_point_for_hit_test(
        m_facts.accumulated_visual_context, position.to_type<float>() * pixel_ratio, document.scroll_state_snapshot());
    if (!result.has_value())
        return position;
    return (*result / pixel_ratio).to_type<CSSPixels>();
}

CSSPixelPoint HitBox::inverse_transform_point(DOM::Document const& document, CSSPixelPoint position) const
{
    if (!has_committed_box())
        return {};
    if (!document.is_rendered())
        return position;
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto result = document.visual_context_tree().inverse_transform_point(m_facts.accumulated_visual_context.spatial, position.to_type<float>() * pixel_ratio);
    return (result / pixel_ratio).to_type<CSSPixels>();
}

DOM::Node* HitTestResult::dom_node() const
{
    if (!document)
        return nullptr;
    return node.resolve(*document).ptr();
}

BoxSlot HitTestResult::scrolling_box() const
{
    if (!document)
        return {};
    return committed_box(*document, hit_node);
}

Optional<DOM::BoundaryPoint> BoundaryIdentity::resolve(DOM::Document& document) const
{
    auto resolved_node = node.resolve(document);
    if (!resolved_node)
        return {};
    return DOM::BoundaryPoint { *resolved_node, offset };
}

GC::Ptr<DOM::Node> CaretPosition::boundary_node() const
{
    if (!document)
        return nullptr;
    return boundary.node.resolve(*document);
}

Optional<DOM::BoundaryPoint> CaretPosition::boundary_point() const
{
    if (!document)
        return {};
    return boundary.resolve(*document);
}

BoxSlot CaretPosition::boundary_box() const
{
    if (!document)
        return {};
    return BoxSlot::bound_to(*document, boundary.node);
}

BoxSlot CaretPosition::box() const
{
    if (!document)
        return {};
    return committed_box(*document, paintable);
}

}
