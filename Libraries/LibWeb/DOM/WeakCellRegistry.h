/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
#include <LibGC/Heap.h>
#include <LibGC/Weak.h>
#include <LibGC/WeakContainer.h>
#include <LibGC/WeakInlines.h>
#include <LibWeb/DOM/RustFFI.h>

namespace Web::DOM {

// A set of cells that a cell keeps weakly, such as a document's live ranges, which libweb_rust reads in place through a
// RustFFI::WeakCellSpan of the owner's core. See Rust/src/dom/weak_cells.rs.
//
// The cells are held by GC::Weak pointers, so the registry keeps none of them alive, and the collector points each at
// the new address of a cell it moves, and nulls one whose cell dies. After every collection, the registry drops the
// null entries. Whenever it changes its entries, it mirrors where they are and how many into the span.
template<typename T>
class WeakCellRegistry final : public GC::WeakContainer {
    AK_MAKE_NONCOPYABLE(WeakCellRegistry);
    AK_MAKE_NONMOVABLE(WeakCellRegistry);

public:
    WeakCellRegistry(GC::Cell const& owner, RustFFI::WeakCellSpan& span)
        : GC::WeakContainer(owner.heap())
        , m_owner(owner)
        , m_span(span)
    {
        mirror();
    }

    void add(T& cell)
    {
        m_cells.append(cell);
        mirror();
    }

    void remove(T& cell)
    {
        auto index = m_cells.find_first_index_if([&](auto const& entry) { return entry.ptr().ptr() == &cell; });
        VERIFY(index.has_value());
        m_cells.remove(*index);
        mirror();
    }

    bool is_empty() const { return m_cells.is_empty(); }

    // NB: The registry names its owner, which is what visits it, so this marks nothing the owner's visit does not.
    void visit_edges(GC::Cell::Visitor& visitor) const { visitor.visit(*m_owner); }

    template<typename Callback>
    void for_each(Callback callback) const
    {
        for (auto const& entry : m_cells) {
            if (auto cell = entry.ptr())
                callback(*cell);
        }
    }

    virtual void remove_dead_cells(Badge<GC::Heap>) override
    {
        m_cells.remove_all_matching([](auto const& entry) { return !entry; });
        mirror();
    }

    virtual GC::Cell const& owner_cell(Badge<GC::Heap>) const override { return *m_owner; }

private:
    void mirror()
    {
        m_span.entries = reinterpret_cast<void const* const*>(m_cells.data());
        m_span.length = m_cells.size();
    }

    GC::Ref<GC::Cell const> m_owner;
    RustFFI::WeakCellSpan& m_span;
    Vector<GC::Weak<T>> m_cells;
};

// NB: The Rust definition of the span asserts the same layout, and reads each entry as a pointer to its GC::WeakImpl.
static_assert(sizeof(RustFFI::WeakCellSpan) == 16 && alignof(RustFFI::WeakCellSpan) == 8);
static_assert(offsetof(RustFFI::WeakCellSpan, entries) == 0);
static_assert(offsetof(RustFFI::WeakCellSpan, length) == 8);
static_assert(sizeof(GC::Weak<GC::Cell>) == sizeof(void*));
static_assert(GC::WeakImpl::value_offset() == RustFFI::WEAK_IMPL_VALUE_OFFSET);

}
