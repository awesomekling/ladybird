/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>

namespace Web::Layout {

class Node;
class TextNode;

// A live row of a document's layout node arena, named by its slot. What the host does to a row that
// only has its style or paint facts written, or its paint damaged, it does through this without
// making the row's shell; a shell is made only for what needs one.
class WEB_API Row {
public:
    Row() = default;
    Row(DOM::Document const&, RustFFI::FfiBoundRow const&);
    // A row whose shell the caller holds. What takes a row takes a shell as well.
    Row(Node const&);

    explicit operator bool() const { return m_document; }

    void* arena_handle() const;
    DOM::Document& document() const { return const_cast<DOM::Document&>(*m_document); }
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
    DOM::Document const* m_document { nullptr };
    Compositing::RustFFI::NodeSlotId m_slot { Compositing::RustFFI::NodeSlotId_INVALID };
    mutable Node* m_shell { nullptr };
    RustFFI::NodeKind m_kind { RustFFI::NodeKind::Unset };
};

WEB_API bool destroy_layout_subtree(Node&);

}
