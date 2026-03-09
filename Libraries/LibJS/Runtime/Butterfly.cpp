/*
 * Copyright (c) 2026, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Checked.h>
#include <LibJS/Runtime/Butterfly.h>
#include <string.h>

namespace JS {

// The butterfly layout in memory (as Value-sized slots) is:
//
//   [named_cap-1 ... 1  0] [header] [0  1 ... vector_len-1]
//                                    ^
//                                    butterfly pointer
//
// Total allocation size = (named_capacity + 1 + vector_length) * sizeof(Value)
// The "+1" is for the ButterflyHeader which occupies one Value-sized slot.
//
// Named property at offset i is at butterfly[-2 - i].
// Indexed element at index j is at butterfly[j].

static Value* allocate_butterfly(u32 named_capacity, u32 vector_length)
{
    size_t total_slots = static_cast<size_t>(named_capacity) + 1 + static_cast<size_t>(vector_length);
    auto* base = static_cast<Value*>(malloc(total_slots * sizeof(Value)));
    VERIFY(base);

    // The butterfly pointer points past the header, at the start of indexed elements.
    auto* butterfly = base + named_capacity + 1;

    // Initialize header.
    auto* hdr = Butterfly::header(butterfly);
    hdr->named_property_capacity = named_capacity;
    hdr->indexed_vector_length = vector_length;

    return butterfly;
}

Value* Butterfly::create_for_named_properties(u32 capacity)
{
    auto* butterfly = allocate_butterfly(capacity, 0);

    // Initialize named property slots to empty.
    for (u32 i = 0; i < capacity; ++i)
        butterfly[-2 - static_cast<i64>(i)] = Value();

    return butterfly;
}

Value* Butterfly::grow_named_properties(Value* old_butterfly, u32 old_capacity, u32 new_capacity)
{
    VERIFY(new_capacity > old_capacity);

    u32 old_vector_length = old_butterfly ? header(old_butterfly)->indexed_vector_length : 0;
    auto* new_butterfly = allocate_butterfly(new_capacity, old_vector_length);

    // Copy existing named properties (they live at negative offsets).
    for (u32 i = 0; i < old_capacity; ++i)
        new_butterfly[-2 - static_cast<i64>(i)] = old_butterfly[-2 - static_cast<i64>(i)];

    // Initialize new named property slots to empty.
    for (u32 i = old_capacity; i < new_capacity; ++i)
        new_butterfly[-2 - static_cast<i64>(i)] = Value();

    // Copy existing indexed elements.
    if (old_vector_length > 0)
        memcpy(new_butterfly, old_butterfly, old_vector_length * sizeof(Value));

    if (old_butterfly)
        destroy(old_butterfly);

    return new_butterfly;
}

Value* Butterfly::create_for_indexed(u32 vector_length)
{
    // Caller is responsible for initializing the indexed slots.
    return allocate_butterfly(0, vector_length);
}

Value* Butterfly::grow_indexed(Value* old_butterfly, u32 old_named_capacity, u32 old_vector_length, u32 new_vector_length)
{
    VERIFY(new_vector_length > old_vector_length);

    // Indexed elements grow on the right side of the butterfly, so realloc
    // can often extend the allocation in place without copying.
    auto* old_base = base(old_butterfly, old_named_capacity);
    size_t new_total_slots = static_cast<size_t>(old_named_capacity) + 1 + static_cast<size_t>(new_vector_length);
    auto* new_base = static_cast<Value*>(realloc(old_base, new_total_slots * sizeof(Value)));
    VERIFY(new_base);

    auto* new_butterfly = new_base + old_named_capacity + 1;
    header(new_butterfly)->indexed_vector_length = new_vector_length;

    // Caller is responsible for initializing new slots.
    return new_butterfly;
}

Value* Butterfly::base(Value* butterfly, u32 named_capacity)
{
    return butterfly - named_capacity - 1;
}

void Butterfly::destroy(Value* butterfly)
{
    if (!butterfly)
        return;
    auto named_capacity = header(butterfly)->named_property_capacity;
    free(base(butterfly, named_capacity));
}

}
