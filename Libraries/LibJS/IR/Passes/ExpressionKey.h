/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Traits.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Key for identifying equivalent expressions.
// Used by GVN and PRE to detect redundant computations.
struct ExpressionKey {
    Opcode opcode;
    ValueIndex operand1 { 0 };
    ValueIndex operand2 { 0 };
    bool has_operand1 { false };
    bool has_operand2 { false };
    u32 extra { 0 }; // For ExtractValue: extract_index

    bool operator==(ExpressionKey const&) const = default;
};

inline Optional<ExpressionKey> make_expression_key(Instruction const& instruction)
{
    if (!instruction.result())
        return {};
    if (instruction.opcode() == Opcode::Phi)
        return {};
    if (!instruction.is_pure())
        return {};
    if (instruction.operand_count() == 0)
        return {};

    ExpressionKey key;
    key.opcode = instruction.opcode();
    if (instruction.operand_count() > 0 && instruction.operand(0)) {
        key.operand1 = instruction.operand(0)->index();
        key.has_operand1 = true;
    }
    if (instruction.operand_count() > 1 && instruction.operand(1)) {
        key.operand2 = instruction.operand(1)->index();
        key.has_operand2 = true;
    }

    if (key.opcode == Opcode::ExtractValue)
        key.extra = instruction.extract_index();

    // Normalize operand order for commutative operations
    if (is_commutative_opcode(key.opcode) && key.has_operand1 && key.has_operand2) {
        if (key.operand1 > key.operand2)
            swap(key.operand1, key.operand2);
    }

    return key;
}

}

template<>
struct AK::Traits<JS::IR::ExpressionKey> : public DefaultTraits<JS::IR::ExpressionKey> {
    static unsigned hash(JS::IR::ExpressionKey const& key)
    {
        return pair_int_hash(
            pair_int_hash(static_cast<u8>(key.opcode), key.extra),
            pair_int_hash(
                key.has_operand1 ? int_hash(static_cast<u32>(key.operand1)) : 0,
                key.has_operand2 ? int_hash(static_cast<u32>(key.operand2)) : 0));
    }
};
