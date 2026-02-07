/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

struct CoalescingMap {
    Vector<ValueIndex> representative_map;

    ValueIndex representative(ValueIndex index) const
    {
        if (representative_map.is_empty())
            return index;
        return representative_map[static_cast<u32>(index)];
    }
};

// CopyCoalescing: Eliminates redundant ParallelCopy entries by merging
// non-interfering source/destination values. After SSA destruction,
// split blocks often contain copies like [v94 <- v17] that prevent
// PostSSACleanup from folding the block. By coalescing v94 and v17
// into a single representative, the copy becomes a no-op and the
// block can be eliminated.
class CopyCoalescing final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "CopyCoalescing"; }

    CoalescingMap coalescing_map() && { return move(m_coalescing_map); }

private:
    CoalescingMap m_coalescing_map;
};

}
