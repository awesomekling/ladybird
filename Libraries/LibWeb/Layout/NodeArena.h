/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Badge.h>
#include <AK/Noncopyable.h>
#include <AK/RefCounted.h>
#include <AK/Types.h>
#include <AK/Vector.h>
#include <AK/WeakPtr.h>
#include <LibGC/Cell.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>

namespace Web::Layout {

class Node;
class NodeArena;
class TextNode;

enum class LayoutUpdatePropagation : u8 {
    ThroughAncestors,
    BoundarySelfOnly,
};

// A live row of a document's layout node arena, named by its slot. What the host does to a row that
// only has its style or paint facts written, or its paint damaged, it does through this without
// making the row's shell; a shell is made only for what needs one.
class WEB_API Row {
public:
    Row() = default;
    Row(NodeArena const&, RustFFI::FfiBoundRow const&);
    // A row whose shell the caller holds. What takes a row takes a shell as well.
    Row(Node const&);

    explicit operator bool() const { return m_arena; }

    NodeArena& arena() const { return const_cast<NodeArena&>(*m_arena); }
    void* arena_handle() const;
    DOM::Document& document() const;
    Compositing::RustFFI::NodeSlotId slot() const { return m_slot; }
    RustFFI::NodeKind kind() const { return m_kind; }
    bool is_text() const { return m_kind == RustFFI::NodeKind::TextNode || m_kind == RustFFI::NodeKind::GeneratedTextNode; }

    // The row's shell if the host made one for it.
    Node* shell_if_made() const { return m_shell; }
    // The row's shell, made now if nothing has asked for it before.
    Node& shell() const;

    Row linked(RustFFI::FfiNodeLink) const;
    u32 flags() const;
    bool has_flag(RustFFI::NodeFlag flag) const { return (flags() & static_cast<u32>(flag)) != 0; }
    bool is_anonymous() const { return has_flag(RustFFI::NodeFlag::Anonymous); }
    // The node the row was built for, as `Node::dom_node_identity()` names it.
    DOM::NodeIdentity dom_node_identity() const;
    // The payloads of the style record the row holds, as its shell reads them if it has one.
    void const* style_payloads() const;
    // The display of the style the row holds.
    CSS::Display display() const;

private:
    NodeArena const* m_arena { nullptr };
    Compositing::RustFFI::NodeSlotId m_slot { Compositing::RustFFI::NodeSlotId_INVALID };
    mutable Node* m_shell { nullptr };
    RustFFI::NodeKind m_kind { RustFFI::NodeKind::Unset };
};

class WEB_API NodeArena : public RefCounted<NodeArena> {
    AK_MAKE_NONCOPYABLE(NodeArena);
    AK_MAKE_NONMOVABLE(NodeArena);

public:
    NodeArena();
    ~NodeArena();

    Compositing::RustFFI::NodeSlotId allocate(RustFFI::FfiNodeConstructionFacts const&);
    void free_subtree(Compositing::RustFFI::NodeSlotId);
    Node* node_if_live(Compositing::RustFFI::NodeSlotId) const;
    // The row `slot` names, found without making a shell for it, if it is still live.
    Row row_if_live(Compositing::RustFFI::NodeSlotId) const;

    // The row of the element or text node with `style_node`, or of its pseudo-element of kind
    // `generated_for`, found without making a shell for it.
    Row bound_row(CSS::StyleNodeID style_node, u8 generated_for = 0) const;
    // The viewport row the document is bound to, found without making a shell for it.
    Row bound_viewport_row() const;
    void* handle() const { return m_render_document.arena; }
    // The name of the document's render state, which the Rendering thread owns.
    RustFFI::DocumentId render_document() const { return m_render_document.document; }

    DOM::Document* document() const { return m_document.ptr(); }
    void set_document(Badge<DOM::Document>, DOM::Document* document) { m_document = document; }

private:
    RustFFI::FfiRenderDocument m_render_document;
    GC::RawPtr<DOM::Document> m_document;
};

WEB_API bool destroy_layout_subtree(Node&);

}
