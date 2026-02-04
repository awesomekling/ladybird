/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Instruction Combining: Combine multiple instructions into simpler forms
// Examples:
//   Not x; Branch x, T, F     →  Branch x, F, T
//   Not (comparison x, y)     →  inverted_comparison x, y
//   Not (Not x)               →  x
//   BitwiseNot (BitwiseNot x) →  x
//   Negate (Negate x)         →  x
//   ToBoolean (ToBoolean x)   →  ToBoolean x
//   ToNumber (ToNumber x)     →  ToNumber x
class InstructionCombining final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "InstructionCombining"; }
};

}
