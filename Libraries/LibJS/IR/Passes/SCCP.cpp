/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <AK/NumericLimits.h>
#include <AK/Queue.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/SCCP.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/AbstractOperations.h>
#include <math.h>

namespace JS::IR {

static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

// Three-level lattice: Unknown (top) -> Constant -> Overdefined (bottom)
enum class LatticeState : u8 {
    Unknown,
    Constant,
    Overdefined,
};

struct LatticeValue {
    LatticeState state { LatticeState::Unknown };
    JS::Value constant_value;

    bool is_unknown() const { return state == LatticeState::Unknown; }
    bool is_constant() const { return state == LatticeState::Constant; }
    bool is_overdefined() const { return state == LatticeState::Overdefined; }

    static LatticeValue unknown() { return { LatticeState::Unknown, {} }; }
    static LatticeValue constant(JS::Value value) { return { LatticeState::Constant, value }; }
    static LatticeValue overdefined() { return { LatticeState::Overdefined, {} }; }

    // Meet two lattice values. Lattice only moves downward.
    LatticeValue meet(LatticeValue const& other) const
    {
        if (is_unknown())
            return other;
        if (other.is_unknown())
            return *this;
        if (is_overdefined() || other.is_overdefined())
            return overdefined();
        // Both are Constant
        if (constant_value.encoded() == other.constant_value.encoded())
            return *this;
        return overdefined();
    }
};

// Pack a CFG edge into a single u64 for the executable-edge set.
static u64 pack_edge(BlockIndex from, BlockIndex to)
{
    return (static_cast<u64>(static_cast<u32>(from)) << 32) | static_cast<u64>(static_cast<u32>(to));
}

// Helpers duplicated from InstCombine for raw JS::Value constant evaluation.
static JS::Value make_int_or_double(i64 result)
{
    if (result >= NumericLimits<i32>::min() && result <= NumericLimits<i32>::max())
        return JS::Value(static_cast<i32>(result));
    return JS::Value(static_cast<double>(result));
}

static Optional<double> numeric_to_double(JS::Value v)
{
    if (v.is_int32())
        return static_cast<double>(v.as_i32());
    if (v.is_double())
        return v.as_double();
    return {};
}

static bool both_numeric(JS::Value a, JS::Value b)
{
    return (a.is_int32() || a.is_double()) && (b.is_int32() || b.is_double());
}

static Optional<i32> constant_to_i32(JS::Value v)
{
    if (v.is_int32())
        return v.as_i32();
    if (v.is_double()) {
        double d = v.as_double();
        if (!isfinite(d) || d == 0.0)
            return static_cast<i32>(0);
        auto int_val = floor(fabs(d));
        if (signbit(d))
            int_val = -int_val;
        auto int32bit = modulo(int_val, 4294967296.0);
        if (int32bit >= 2147483648.0)
            int32bit -= 4294967296.0;
        return static_cast<i32>(int32bit);
    }
    if (v.is_boolean())
        return v.as_bool() ? 1 : 0;
    if (v.is_null() || v.is_undefined())
        return static_cast<i32>(0);
    return {};
}

// Returns the truthiness of a JS::Value if determinable.
static Optional<bool> value_truthiness(JS::Value v)
{
    if (v.is_boolean())
        return v.as_bool();
    if (v.is_int32())
        return v.as_i32() != 0;
    if (v.is_undefined() || v.is_null())
        return false;
    if (v.is_double())
        return !v.is_nan() && v.as_double() != 0;
    return {};
}

// Is this JS::Value a safe primitive for effect refinement?
static bool is_safe_primitive_value(JS::Value v)
{
    return v.is_undefined() || v.is_null() || v.is_boolean() || v.is_int32() || v.is_double() || v.is_string();
}

// Try to evaluate an instruction with constant operands.
// Returns the result JS::Value if foldable, empty otherwise.
static Optional<JS::Value> try_evaluate(Opcode opcode, ReadonlySpan<JS::Value> operands)
{
    auto nops = operands.size();

    switch (opcode) {
    case Opcode::Add:
        if (nops == 2 && operands[0].is_int32() && operands[1].is_int32()) {
            i64 lhs = operands[0].as_i32();
            i64 rhs = operands[1].as_i32();
            return make_int_or_double(lhs + rhs);
        }
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) + *numeric_to_double(operands[1]));
        break;

    case Opcode::Sub:
        if (nops == 2 && operands[0].is_int32() && operands[1].is_int32()) {
            i64 lhs = operands[0].as_i32();
            i64 rhs = operands[1].as_i32();
            return make_int_or_double(lhs - rhs);
        }
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) - *numeric_to_double(operands[1]));
        break;

    case Opcode::Mul:
        if (nops == 2 && operands[0].is_int32() && operands[1].is_int32()) {
            i64 lhs = operands[0].as_i32();
            i64 rhs = operands[1].as_i32();
            return make_int_or_double(lhs * rhs);
        }
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) * *numeric_to_double(operands[1]));
        break;

    case Opcode::Div:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) / *numeric_to_double(operands[1]));
        break;

    case Opcode::Mod:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(fmod(*numeric_to_double(operands[0]), *numeric_to_double(operands[1])));
        break;

    case Opcode::Exp:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(pow(*numeric_to_double(operands[0]), *numeric_to_double(operands[1])));
        break;

    case Opcode::LessThan:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) < *numeric_to_double(operands[1]));
        break;

    case Opcode::LessThanEquals:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) <= *numeric_to_double(operands[1]));
        break;

    case Opcode::GreaterThan:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) > *numeric_to_double(operands[1]));
        break;

    case Opcode::GreaterThanEquals:
        if (nops == 2 && both_numeric(operands[0], operands[1]))
            return JS::Value(*numeric_to_double(operands[0]) >= *numeric_to_double(operands[1]));
        break;

    case Opcode::StrictlyEquals:
    case Opcode::StrictlyInequals: {
        if (nops != 2)
            break;
        auto const& lhs = operands[0];
        auto const& rhs = operands[1];
        bool equals = false;
        bool can_determine = false;

        if (both_numeric(lhs, rhs)) {
            equals = *numeric_to_double(lhs) == *numeric_to_double(rhs);
            can_determine = true;
        } else if (lhs.is_boolean() && rhs.is_boolean()) {
            equals = lhs.as_bool() == rhs.as_bool();
            can_determine = true;
        } else if ((lhs.is_null() && rhs.is_null()) || (lhs.is_undefined() && rhs.is_undefined())) {
            equals = true;
            can_determine = true;
        } else if (lhs.is_string() && rhs.is_string()) {
            equals = lhs.as_string().utf8_string_view() == rhs.as_string().utf8_string_view();
            can_determine = true;
        } else if ((lhs.is_null() || lhs.is_undefined() || lhs.is_boolean() || lhs.is_number())
            && (rhs.is_null() || rhs.is_undefined() || rhs.is_boolean() || rhs.is_number())) {
            equals = false;
            can_determine = true;
        }

        if (can_determine) {
            bool result = (opcode == Opcode::StrictlyEquals) ? equals : !equals;
            return JS::Value(result);
        }
        break;
    }

    case Opcode::LooselyEquals:
    case Opcode::LooselyInequals: {
        if (nops != 2)
            break;
        auto const& lhs = operands[0];
        auto const& rhs = operands[1];
        bool equals = false;
        bool can_determine = false;

        auto is_nullish = [](JS::Value v) { return v.is_null() || v.is_undefined(); };

        if (is_nullish(lhs) && is_nullish(rhs)) {
            equals = true;
            can_determine = true;
        } else if (is_nullish(lhs) || is_nullish(rhs)) {
            equals = false;
            can_determine = true;
        } else if (both_numeric(lhs, rhs)) {
            equals = *numeric_to_double(lhs) == *numeric_to_double(rhs);
            can_determine = true;
        } else if (lhs.is_boolean() && rhs.is_boolean()) {
            equals = lhs.as_bool() == rhs.as_bool();
            can_determine = true;
        } else if (lhs.is_string() && rhs.is_string()) {
            equals = lhs.as_string().utf8_string_view() == rhs.as_string().utf8_string_view();
            can_determine = true;
        } else if (lhs.is_boolean() && (rhs.is_int32() || rhs.is_double())) {
            equals = static_cast<double>(lhs.as_bool() ? 1 : 0) == *numeric_to_double(rhs);
            can_determine = true;
        } else if ((lhs.is_int32() || lhs.is_double()) && rhs.is_boolean()) {
            equals = *numeric_to_double(lhs) == static_cast<double>(rhs.as_bool() ? 1 : 0);
            can_determine = true;
        }

        if (can_determine) {
            bool result = (opcode == Opcode::LooselyEquals) ? equals : !equals;
            return JS::Value(result);
        }
        break;
    }

    case Opcode::BitwiseAnd:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value())
                return JS::Value(*lhs & *rhs);
        }
        break;

    case Opcode::BitwiseOr:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value())
                return JS::Value(*lhs | *rhs);
        }
        break;

    case Opcode::BitwiseXor:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value())
                return JS::Value(*lhs ^ *rhs);
        }
        break;

    case Opcode::BitwiseNot:
        if (nops == 1) {
            auto val = constant_to_i32(operands[0]);
            if (val.has_value())
                return JS::Value(~*val);
        }
        break;

    case Opcode::LeftShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value()) {
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                return JS::Value(*lhs << shift_count);
            }
        }
        break;

    case Opcode::RightShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value()) {
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                return JS::Value(*lhs >> shift_count);
            }
        }
        break;

    case Opcode::UnsignedRightShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(operands[0]);
            auto rhs = constant_to_i32(operands[1]);
            if (lhs.has_value() && rhs.has_value()) {
                u32 unsigned_lhs = static_cast<u32>(*lhs);
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                u32 result = unsigned_lhs >> shift_count;
                if (result <= static_cast<u32>(NumericLimits<i32>::max()))
                    return JS::Value(static_cast<i32>(result));
                return JS::Value(static_cast<double>(result));
            }
        }
        break;

    case Opcode::Negate:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32()) {
                i32 val = v.as_i32();
                if (val == 0)
                    return JS::Value(-0.0);
                return make_int_or_double(-static_cast<i64>(val));
            }
            if (v.is_double())
                return JS::Value(-v.as_double());
            if (v.is_boolean())
                return v.as_bool() ? JS::Value(-1) : JS::Value(-0.0);
            if (v.is_null())
                return JS::Value(-0.0);
            if (v.is_undefined())
                return JS::Value(js_nan());
        }
        break;

    case Opcode::UnaryPlus:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32() || v.is_double())
                return v;
            if (v.is_boolean())
                return JS::Value(v.as_bool() ? 1 : 0);
            if (v.is_null())
                return JS::Value(0);
            if (v.is_undefined())
                return JS::Value(js_nan());
        }
        break;

    case Opcode::Increment:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32())
                return make_int_or_double(static_cast<i64>(v.as_i32()) + 1);
            if (v.is_double())
                return JS::Value(v.as_double() + 1);
        }
        break;

    case Opcode::Decrement:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32())
                return make_int_or_double(static_cast<i64>(v.as_i32()) - 1);
            if (v.is_double())
                return JS::Value(v.as_double() - 1);
        }
        break;

    case Opcode::ToInt32:
        if (nops == 1) {
            auto val = constant_to_i32(operands[0]);
            if (val.has_value())
                return JS::Value(*val);
        }
        break;

    case Opcode::ToNumber:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32() || v.is_double())
                return v;
            if (v.is_boolean())
                return JS::Value(v.as_bool() ? 1 : 0);
            if (v.is_undefined())
                return JS::Value(js_nan());
            if (v.is_null())
                return JS::Value(0);
        }
        break;

    case Opcode::ToNumeric:
        if (nops == 1) {
            auto const& v = operands[0];
            if (v.is_int32() || v.is_double() || v.is_bigint())
                return v;
            if (v.is_boolean())
                return JS::Value(v.as_bool() ? 1 : 0);
            if (v.is_undefined())
                return JS::Value(js_nan());
            if (v.is_null())
                return JS::Value(0);
        }
        break;

    case Opcode::ToBoolean:
        if (nops == 1) {
            if (auto truthiness = value_truthiness(operands[0]); truthiness.has_value())
                return JS::Value(*truthiness);
        }
        break;

    case Opcode::Not:
        if (nops == 1) {
            if (auto truthiness = value_truthiness(operands[0]); truthiness.has_value())
                return JS::Value(!*truthiness);
        }
        break;

    case Opcode::IsUndefined:
        if (nops == 1)
            return JS::Value(operands[0].is_undefined());
        break;

    case Opcode::IsNullish:
        if (nops == 1) {
            auto const& v = operands[0];
            return JS::Value(v.is_null() || v.is_undefined());
        }
        break;

    default:
        break;
    }

    return {};
}

// Check if an instruction is safe to fold given constant operand values.
// An instruction is safe to fold if:
// 1. It has no effects (pure), OR
// 2. Its refinement is PureOnSafePrimitives and all operand constants are safe primitives
static bool is_safe_to_fold(Opcode opcode, ReadonlySpan<JS::Value> operands)
{
    auto const& traits = opcode_traits(opcode);

    if (traits.effects == Effects::None)
        return true;

    if (traits.refinement == EffectRefinement::PureOnSafePrimitives) {
        for (auto const& value : operands) {
            if (!is_safe_primitive_value(value))
                return false;
        }
        return true;
    }

    return false;
}

PreservedAnalyses SCCP::run(Function& function, PassManager&)
{
    auto value_count = function.values().size();

    // Lattice state for each SSA value
    Vector<LatticeValue> lattice;
    lattice.resize(value_count);

    // Executable edges and blocks
    HashTable<u64> executable_edges;
    HashTable<BasicBlock*> executable_blocks;

    // Worklists
    Queue<BasicBlock*> cfg_worklist;
    Queue<Value*> ssa_worklist;

    // Initialize lattice for all values
    for (auto const& value : function.values()) {
        auto index = to_index(value->index());
        switch (value->kind()) {
        case Value::Kind::Parameter:
        case Value::Kind::This:
            lattice[index] = LatticeValue::overdefined();
            break;
        case Value::Kind::Constant:
            lattice[index] = LatticeValue::constant(value->constant_value());
            break;
        case Value::Kind::Instruction:
            lattice[index] = LatticeValue::unknown();
            break;
        }
    }

    // Seed the entry block
    auto* entry = function.entry_block();
    if (!entry)
        return PreservedAnalyses::all();

    cfg_worklist.enqueue(entry);
    executable_blocks.set(entry);

    // Helper: get lattice value for an IR value
    auto get_lattice = [&](Value* value) -> LatticeValue {
        if (!value)
            return LatticeValue::overdefined();
        return lattice[to_index(value->index())];
    };

    // Helper: update a value's lattice and add to SSA worklist if changed
    auto update_lattice = [&](Value* value, LatticeValue new_lattice) {
        auto index = to_index(value->index());
        auto& current = lattice[index];

        // Lattice only moves downward: Unknown -> Constant -> Overdefined
        auto merged = current.meet(new_lattice);
        if (merged.state != current.state || (merged.is_constant() && current.is_constant() && merged.constant_value.encoded() != current.constant_value.encoded())) {
            current = merged;
            ssa_worklist.enqueue(value);
        }
    };

    // Helper: evaluate a phi instruction
    auto evaluate_phi = [&](PhiInstruction& phi) {
        LatticeValue result = LatticeValue::unknown();

        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            auto predecessor_index = phi.incoming_block(i);

            // Only consider executable edges
            if (!executable_edges.contains(pack_edge(predecessor_index, phi.parent_block()->index())))
                continue;

            auto* incoming_value = phi.incoming_value(i);
            auto incoming_lattice = get_lattice(incoming_value);
            result = result.meet(incoming_lattice);

            if (result.is_overdefined())
                break;
        }

        if (phi.result())
            update_lattice(phi.result(), result);
    };

    // Helper: evaluate a non-phi instruction
    auto evaluate_instruction = [&](Instruction& instruction) {
        if (instruction.opcode() == Opcode::Phi)
            return;
        if (!instruction.result())
            return;

        // Terminators with results (e.g. Yield) produce values we can't
        // statically determine — mark them Overdefined.
        if (instruction.is_terminator()) {
            update_lattice(instruction.result(), LatticeValue::overdefined());
            return;
        }

        // Move just propagates its operand's lattice
        if (instruction.opcode() == Opcode::Move) {
            if (instruction.operand_count() == 1 && instruction.operand(0))
                update_lattice(instruction.result(), get_lattice(instruction.operand(0)));
            else
                update_lattice(instruction.result(), LatticeValue::overdefined());
            return;
        }

        // Check if all operands are resolved (not Unknown)
        bool any_unknown = false;
        bool all_constant = true;
        Vector<JS::Value> constant_operands;
        constant_operands.ensure_capacity(instruction.operand_count());

        for (size_t i = 0; i < instruction.operand_count(); ++i) {
            auto operand_lattice = get_lattice(instruction.operand(i));
            if (operand_lattice.is_unknown()) {
                any_unknown = true;
                break;
            }
            if (!operand_lattice.is_constant()) {
                all_constant = false;
                break;
            }
            constant_operands.append(operand_lattice.constant_value);
        }

        // If any operand is still Unknown, leave this value as Unknown
        if (any_unknown)
            return;

        // If not all operands are constant, result is Overdefined
        if (!all_constant) {
            update_lattice(instruction.result(), LatticeValue::overdefined());
            return;
        }

        // All operands are constant — try to fold
        if (!is_safe_to_fold(instruction.opcode(), constant_operands)) {
            update_lattice(instruction.result(), LatticeValue::overdefined());
            return;
        }

        auto folded = try_evaluate(instruction.opcode(), constant_operands);
        if (folded.has_value())
            update_lattice(instruction.result(), LatticeValue::constant(*folded));
        else
            update_lattice(instruction.result(), LatticeValue::overdefined());
    };

    // Helper: process a terminator for CFG edge discovery
    auto process_terminator = [&](BasicBlock& block) {
        auto* terminator = block.terminator();
        if (!terminator)
            return;

        auto mark_edge = [&](BasicBlock& target) {
            auto edge = pack_edge(block.index(), target.index());
            if (executable_edges.set(edge) == AK::HashSetResult::InsertedNewEntry) {
                if (!executable_blocks.contains(&target)) {
                    executable_blocks.set(&target);
                    cfg_worklist.enqueue(&target);
                } else {
                    // Block already executable but new edge — re-evaluate phis
                    target.for_each_phi([&](PhiInstruction& phi) {
                        evaluate_phi(phi);
                    });
                }
            }
        };

        if (terminator->opcode() == Opcode::Branch) {
            auto condition_lattice = get_lattice(terminator->operand(0));

            if (condition_lattice.is_constant()) {
                auto truthiness = value_truthiness(condition_lattice.constant_value);
                if (truthiness.has_value()) {
                    // Only mark the taken edge
                    if (*truthiness) {
                        if (auto* target = terminator->true_target())
                            mark_edge(*target);
                    } else {
                        if (auto* target = terminator->false_target())
                            mark_edge(*target);
                    }
                } else {
                    // Truthiness not determinable — mark both edges
                    if (auto* target = terminator->true_target())
                        mark_edge(*target);
                    if (auto* target = terminator->false_target())
                        mark_edge(*target);
                }
            } else {
                // Condition unknown or overdefined — mark both edges
                if (auto* target = terminator->true_target())
                    mark_edge(*target);
                if (auto* target = terminator->false_target())
                    mark_edge(*target);
            }
        } else {
            // Unconditional jump or other terminator — mark all successors
            if (auto* target = terminator->true_target())
                mark_edge(*target);
            if (auto* target = terminator->false_target())
                mark_edge(*target);
        }

        // Always mark exception handler and finalizer edges reachable.
        // These are implicit edges not reflected in the terminator targets.
        if (auto* handler = block.exception_handler())
            mark_edge(*handler);
        if (auto* finalizer = block.finalizer())
            mark_edge(*finalizer);
    };

    // Main solving loop
    while (!cfg_worklist.is_empty() || !ssa_worklist.is_empty()) {
        // Process CFG worklist: visit newly executable blocks
        while (!cfg_worklist.is_empty()) {
            auto* block = cfg_worklist.dequeue();

            // Evaluate all phis first
            block->for_each_phi([&](PhiInstruction& phi) {
                evaluate_phi(phi);
            });

            // Evaluate all non-phi, non-terminator instructions
            block->for_each_instruction([&](Instruction& instruction) {
                evaluate_instruction(instruction);
            });

            // Process terminator edges
            process_terminator(*block);
        }

        // Process SSA worklist: re-evaluate users of changed values
        while (!ssa_worklist.is_empty()) {
            auto* value = ssa_worklist.dequeue();

            for (auto const& use : value->uses()) {
                auto* user_instruction = function.instruction_by_index(use.instruction);
                auto* user_block = user_instruction->parent_block();

                // Only re-evaluate in executable blocks
                if (!executable_blocks.contains(user_block))
                    continue;

                if (user_instruction->opcode() == Opcode::Phi)
                    evaluate_phi(static_cast<PhiInstruction&>(*user_instruction));
                else
                    evaluate_instruction(*user_instruction);

                // If this is a terminator's operand, re-process its edges
                if (user_instruction->is_terminator())
                    process_terminator(*user_block);
            }
        }
    }

    // Rewrite phase: replace constants and fold branches
    bool changed = false;

    // 1. Replace lattice-Constant values with IR constants.
    //    Collect replacements first because create_constant() may grow the
    //    values vector, invalidating Value* pointers during iteration.
    struct Replacement {
        ValueIndex target;
        JS::Value constant_value;
    };
    Vector<Replacement> replacements;

    for (auto const& value : function.values()) {
        if (value->kind() != Value::Kind::Instruction)
            continue;

        auto const& lattice_value = lattice[to_index(value->index())];
        if (!lattice_value.is_constant())
            continue;

        if (value->uses().is_empty())
            continue;

        replacements.append({ value->index(), lattice_value.constant_value });
    }

    for (auto const& replacement : replacements) {
        auto& constant = function.create_constant(replacement.constant_value);
        auto* target = function.values()[to_index(replacement.target)].ptr();
        target->replace_all_uses_with(&constant);
        changed = true;
    }

    // 2. Fold constant branches to jumps
    for (auto& block : function.basic_blocks()) {
        if (!executable_blocks.contains(block.ptr()))
            continue;

        auto* terminator = block->terminator();
        if (!terminator || terminator->opcode() != Opcode::Branch)
            continue;

        auto* condition = terminator->operand(0);
        if (!condition)
            continue;

        auto truthiness = condition->constant_truthiness();
        if (!truthiness.has_value())
            continue;

        auto* target = *truthiness ? terminator->true_target() : terminator->false_target();
        auto* not_taken = *truthiness ? terminator->false_target() : terminator->true_target();

        if (target)
            CFG::replace_branch_with_jump(*block, *target, not_taken);

        changed = true;
    }

    // 3. Remove unreachable blocks
    HashTable<BasicBlock*> blocks_to_remove;
    for (auto& block : function.basic_blocks()) {
        if (!executable_blocks.contains(block.ptr()) && block.ptr() != entry)
            blocks_to_remove.set(block.ptr());
    }

    if (!blocks_to_remove.is_empty()) {
        CFG::remove_blocks(function, blocks_to_remove);
        changed = true;
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
