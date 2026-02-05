/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/Export.h>
#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

enum class VerifierMode {
    // Structural checks only. Tolerates unreachable blocks and dead code
    // (normal intermediate states during optimization). Only verifies
    // reachable code for SSA dominance and operand validity.
    InterPass,

    // All structural checks plus cleanliness checks: no unreachable blocks,
    // no empty blocks, block index uniqueness. Use after optimization is complete.
    Full,
};

// IR Verifier: Validate CFG invariants and SSA properties
// This is a debugging/validation pass that does not modify the IR.
// Returns true if the IR is valid, crashes or returns false on errors.
class JS_API Verifier final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "Verifier"; }

    // Run verification and return true if valid, false otherwise
    // When crash_on_error is true (default), crashes immediately on first error
    static bool verify(Function&, VerifierMode = VerifierMode::InterPass, bool crash_on_error = true);
};

}
