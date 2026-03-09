/*
 * Copyright (c) 2026, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>
#include <LibJS/Runtime/Value.h>

namespace JS {

struct ButterflyHeader {
    u32 named_property_capacity { 0 };
    u32 indexed_vector_length { 0 };
};

static_assert(sizeof(ButterflyHeader) == 8);

// Butterfly memory layout:
//
//   [prop_N-1 ... prop_1  prop_0] [ButterflyHeader (8 bytes)] [elem_0  elem_1 ... elem_M-1]
//                                                              ^
//                                                              m_butterfly points here
//
// Named properties at negative offsets: m_butterfly[-2 - offset]
//   (offset 0 is closest to center, offset N-1 is farthest left)
//   The -2 accounts for the 8-byte header = 1 Value slot between props and elements.
//   Wait, actually the header is between the named props and the pointer, so:
//   header is at ((ButterflyHeader*)m_butterfly)[-1]
//   Named prop at offset i is at m_butterfly[-2 - i]
//     because m_butterfly[-1] overlaps the header.
//
// Indexed elements at positive offsets: m_butterfly[index]

class Butterfly {
public:
    // Allocate a butterfly with space for named properties only.
    static Value* create_for_named_properties(u32 capacity);

    // Grow the named property side of a butterfly, preserving existing data.
    // Returns a new butterfly pointer (old one is freed).
    static Value* grow_named_properties(Value* old_butterfly, u32 old_capacity, u32 new_capacity);

    // Allocate a butterfly with space for indexed elements only.
    static Value* create_for_indexed(u32 vector_length);

    // Grow the indexed side of a butterfly, preserving both named and indexed data.
    // Returns a new butterfly pointer (old one is freed).
    static Value* grow_indexed(Value* old_butterfly, u32 old_named_capacity, u32 old_vector_length, u32 new_vector_length);

    // Get the header from a butterfly pointer.
    static ButterflyHeader* header(Value* butterfly)
    {
        return reinterpret_cast<ButterflyHeader*>(butterfly) - 1;
    }

    static ButterflyHeader const* header(Value const* butterfly)
    {
        return reinterpret_cast<ButterflyHeader const*>(butterfly) - 1;
    }

    // Compute the base pointer (start of malloc allocation) for freeing.
    static Value* base(Value* butterfly, u32 named_capacity);

    // Free a butterfly allocation.
    static void destroy(Value* butterfly);
};

}
