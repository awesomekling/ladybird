/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashTable.h>
#include <LibGC/Cell.h>
#include <LibGC/NanBoxedValue.h>
#include <LibGC/Ptr.h>

namespace GC {

struct RootsCollector : public Cell::Visitor {
    RootsCollector()
        : Visitor(Kind::RootsCollector, this)
    {
    }

    void visit_impl(Cell& cell)
    {
        roots.set(&cell);
    }

    void visit_impl(ReadonlySpan<NanBoxedValue> values)
    {
        for (auto const& value : values) {
            if (value.is_cell())
                roots.set(value.as_cell());
        }
    }

    void visit_possible_values(ReadonlyBytes)
    {
        VERIFY_NOT_REACHED();
    }

    HashTable<Ptr<Cell>> roots;
};

}
