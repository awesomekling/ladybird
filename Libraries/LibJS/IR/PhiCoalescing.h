/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API PhiCoalescing {
public:
    explicit PhiCoalescing(Function const&);

    // Find the coalescing representative for a value.
    // Returns the value itself if not coalesced.
    Value const* representative(Value const&) const;

private:
    void compute();
    Value const* find_representative(Value const* v) const;

    Function const& m_function;
    mutable Vector<Value const*> m_representative; // mutable for path compression
};

}
