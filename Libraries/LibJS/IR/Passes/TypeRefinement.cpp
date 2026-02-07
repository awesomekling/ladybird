/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Vector.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Passes/TypeRefinement.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/PrimitiveString.h>
#include <LibJS/Runtime/VM.h>

namespace JS::IR {

static GC::Ptr<PrimitiveString> typeof_result_for_type(Type type)
{
    auto& vm = VM::the();
    switch (type) {
    case Type::Int32:
    case Type::Number:
        return vm.cached_strings.number;
    case Type::Boolean:
        return vm.cached_strings.boolean;
    case Type::Undefined:
        return vm.cached_strings.undefined;
    case Type::Null:
        return vm.cached_strings.object;
    case Type::String:
        return vm.cached_strings.string;
    case Type::Symbol:
        return vm.cached_strings.symbol;
    case Type::BigInt:
        return vm.cached_strings.bigint;
    case Type::Function:
        return vm.cached_strings.function;
    case Type::Array:
        return vm.cached_strings.object;
    default:
        return nullptr;
    }
}

// Check if all types in a mask produce the same typeof string.
// Returns the shared PrimitiveString if so, nullptr otherwise.
static GC::Ptr<PrimitiveString> typeof_result_for_mask(TypeMask mask)
{
    if (mask.is_empty())
        return nullptr;

    GC::Ptr<PrimitiveString> result;
    for (u8 i = 1; i <= 11; ++i) {
        auto type = static_cast<Type>(i);
        if (!mask.contains(type))
            continue;
        auto typeof_string = typeof_result_for_type(type);
        if (!typeof_string)
            return nullptr;
        if (!result)
            result = typeof_string;
        else if (result != typeof_string)
            return nullptr;
    }
    return result;
}

// Map a typeof string to the TypeMask of types that produce it.
static TypeMask typeof_string_to_type_mask(StringView typeof_string)
{
    if (typeof_string == "number"sv)
        return TypeMask::for_type(Type::Int32) | TypeMask::for_type(Type::Number);
    if (typeof_string == "string"sv)
        return TypeMask::for_type(Type::String);
    if (typeof_string == "boolean"sv)
        return TypeMask::for_type(Type::Boolean);
    if (typeof_string == "undefined"sv)
        return TypeMask::for_type(Type::Undefined);
    if (typeof_string == "object"sv)
        return TypeMask::for_type(Type::Null) | TypeMask::for_type(Type::Object) | TypeMask::for_type(Type::Array);
    if (typeof_string == "function"sv)
        return TypeMask::for_type(Type::Function);
    if (typeof_string == "symbol"sv)
        return TypeMask::for_type(Type::Symbol);
    if (typeof_string == "bigint"sv)
        return TypeMask::for_type(Type::BigInt);
    return TypeMask::all();
}

struct ScopedFact {
    ValueIndex value;
    TypeMask previous_mask;
};

PreservedAnalyses TypeRefinement::run(Function& function, PassManager& pass_manager)
{
    if (!function.entry_block())
        return PreservedAnalyses::all();

    bool changed = false;
    auto const& dominators = pass_manager.dominator_tree(function);
    HashTable<Instruction*> dead_instructions;

    // Map from value index to its refined type mask.
    HashMap<ValueIndex, TypeMask> type_facts;

    // Seed type facts from values with known types.
    for (auto const& value : function.values()) {
        if (value->type() != Type::Unknown)
            type_facts.set(value->index(), TypeMask::for_type(value->type()));
    }

    auto get_mask = [&](Value* value) -> TypeMask {
        if (!value)
            return TypeMask::all();
        auto it = type_facts.find(value->index());
        if (it != type_facts.end())
            return it->value;
        return TypeMask::all();
    };

    auto process_block = [&](auto& self, BasicBlock* block) -> void {
        Vector<ScopedFact> scoped_facts;

        auto set_fact = [&](ValueIndex index, TypeMask mask) {
            auto it = type_facts.find(index);
            TypeMask previous = (it != type_facts.end()) ? it->value : TypeMask::all();
            scoped_facts.append({ index, previous });
            type_facts.set(index, mask);
        };

        // Apply optimizations using current facts.
        block->for_each_instruction([&](Instruction& instruction) {
            switch (instruction.opcode()) {

            // Typeof fold: if all types in the mask produce the same typeof string,
            // replace the Typeof result with that constant string.
            case Opcode::Typeof: {
                if (!instruction.result() || instruction.result()->uses().is_empty())
                    break;
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto mask = get_mask(instruction.operand(0));
                if (mask == TypeMask::all())
                    break;
                auto typeof_string = typeof_result_for_mask(mask);
                if (!typeof_string)
                    break;
                auto& constant = function.create_constant(JS::Value(typeof_string));
                instruction.result()->replace_all_uses_with(&constant);
                changed = true;
                break;
            }

            // ThrowIfNullish removal: if the value is known non-nullish, mark for removal.
            case Opcode::ThrowIfNullish: {
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto mask = get_mask(instruction.operand(0));
                if (mask.excludes_nullish()) {
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            // StrictlyEquals / StrictlyInequals / LooselyEquals / LooselyInequals
            // fold when comparing against null or undefined with known masks.
            case Opcode::StrictlyEquals:
            case Opcode::StrictlyInequals:
            case Opcode::LooselyEquals:
            case Opcode::LooselyInequals: {
                if (!instruction.result() || instruction.result()->uses().is_empty())
                    break;
                if (instruction.operand_count() < 2)
                    break;
                auto* lhs = instruction.operand(0);
                auto* rhs = instruction.operand(1);
                if (!lhs || !rhs)
                    break;

                bool is_equals = (instruction.opcode() == Opcode::StrictlyEquals
                    || instruction.opcode() == Opcode::LooselyEquals);
                bool is_loose = (instruction.opcode() == Opcode::LooselyEquals
                    || instruction.opcode() == Opcode::LooselyInequals);

                // Check for comparison with null or undefined constant.
                bool rhs_is_null = rhs->is_constant() && rhs->constant_value().is_null();
                bool rhs_is_undefined = rhs->is_constant() && rhs->constant_value().is_undefined();
                if (rhs_is_null || rhs_is_undefined) {
                    auto mask = get_mask(lhs);

                    // For loose equality, null == undefined is true, so both
                    // null and undefined match. For strict, only the exact type.
                    TypeMask matching_mask;
                    if (is_loose)
                        matching_mask = TypeMask::nullish();
                    else if (rhs_is_null)
                        matching_mask = TypeMask::for_type(Type::Null);
                    else
                        matching_mask = TypeMask::for_type(Type::Undefined);

                    if ((mask & ~matching_mask).is_empty()) {
                        // All possible types match — fold to true/false.
                        auto& constant = function.create_constant(JS::Value(is_equals));
                        instruction.result()->replace_all_uses_with(&constant);
                        changed = true;
                    } else if ((mask & matching_mask).is_empty()) {
                        // No possible type matches — fold to false/true.
                        auto& constant = function.create_constant(JS::Value(!is_equals));
                        instruction.result()->replace_all_uses_with(&constant);
                        changed = true;
                    }
                }
                break;
            }

            // IsNullish fold: known non-nullish -> false, known nullish -> true.
            case Opcode::IsNullish: {
                if (!instruction.result() || instruction.result()->uses().is_empty())
                    break;
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto mask = get_mask(instruction.operand(0));
                if (mask.excludes_nullish()) {
                    auto& constant = function.create_constant(JS::Value(false));
                    instruction.result()->replace_all_uses_with(&constant);
                    changed = true;
                } else if ((mask & ~TypeMask::nullish()).is_empty()) {
                    auto& constant = function.create_constant(JS::Value(true));
                    instruction.result()->replace_all_uses_with(&constant);
                    changed = true;
                }
                break;
            }

            // IsUndefined fold: known non-undefined -> false, known undefined -> true.
            case Opcode::IsUndefined: {
                if (!instruction.result() || instruction.result()->uses().is_empty())
                    break;
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto mask = get_mask(instruction.operand(0));
                if (!mask.contains(Type::Undefined)) {
                    auto& constant = function.create_constant(JS::Value(false));
                    instruction.result()->replace_all_uses_with(&constant);
                    changed = true;
                } else if (mask == TypeMask::for_type(Type::Undefined)) {
                    auto& constant = function.create_constant(JS::Value(true));
                    instruction.result()->replace_all_uses_with(&constant);
                    changed = true;
                }
                break;
            }

            default:
                break;
            }
        });

        // Analyze the block's terminator for type guards to install on successors.
        auto* terminator = block->terminator();
        if (terminator && terminator->opcode() == Opcode::Branch) {
            auto* branch = static_cast<BranchInstruction*>(terminator);
            auto* true_target = branch->true_target();
            auto* false_target = branch->false_target();

            // Don't install facts if both targets are the same block.
            if (true_target != false_target) {
                auto* condition = branch->condition();
                if (condition) {
                    auto* condition_instruction = condition->defining_instruction();
                    if (condition_instruction) {
                        TypeMask true_mask = TypeMask::all();
                        TypeMask false_mask = TypeMask::all();
                        Value* guarded_value = nullptr;

                        auto extract_typeof_guard = [&](Instruction* instruction) {
                            // Pattern: Equals(Typeof(v), "typename") or Inequals(Typeof(v), "typename")
                            auto opcode = instruction->opcode();
                            bool is_equals = (opcode == Opcode::StrictlyEquals || opcode == Opcode::LooselyEquals);
                            bool is_inequals = (opcode == Opcode::StrictlyInequals || opcode == Opcode::LooselyInequals);
                            if (!is_equals && !is_inequals)
                                return;

                            if (instruction->operand_count() < 2)
                                return;
                            auto* lhs = instruction->operand(0);
                            auto* rhs = instruction->operand(1);
                            if (!lhs || !rhs)
                                return;

                            // One side should be Typeof, the other a string constant.
                            Value* typeof_operand = nullptr;
                            Value* string_constant = nullptr;

                            if (lhs->defining_instruction() && lhs->defining_instruction()->opcode() == Opcode::Typeof
                                && rhs->is_constant() && rhs->constant_value().is_string()) {
                                typeof_operand = lhs->defining_instruction()->operand(0);
                                string_constant = rhs;
                            } else if (rhs->defining_instruction() && rhs->defining_instruction()->opcode() == Opcode::Typeof
                                && lhs->is_constant() && lhs->constant_value().is_string()) {
                                typeof_operand = rhs->defining_instruction()->operand(0);
                                string_constant = lhs;
                            }

                            if (!typeof_operand || !string_constant)
                                return;

                            auto typeof_string = string_constant->constant_value().as_string().utf8_string_view();
                            auto matching_mask = typeof_string_to_type_mask(typeof_string);

                            guarded_value = typeof_operand;
                            auto current = get_mask(typeof_operand);
                            if (is_equals) {
                                true_mask = current & matching_mask;
                                false_mask = current.exclude(matching_mask);
                            } else {
                                true_mask = current.exclude(matching_mask);
                                false_mask = current & matching_mask;
                            }
                        };

                        auto extract_nullish_guard = [&](Instruction* instruction) {
                            if (instruction->opcode() == Opcode::IsNullish) {
                                if (instruction->operand_count() == 0 || !instruction->operand(0))
                                    return;
                                guarded_value = instruction->operand(0);
                                auto current = get_mask(guarded_value);
                                true_mask = current & TypeMask::nullish();
                                false_mask = current.exclude(TypeMask::nullish());
                            } else if (instruction->opcode() == Opcode::IsUndefined) {
                                if (instruction->operand_count() == 0 || !instruction->operand(0))
                                    return;
                                guarded_value = instruction->operand(0);
                                auto current = get_mask(guarded_value);
                                auto undefined_mask = TypeMask::for_type(Type::Undefined);
                                true_mask = current & undefined_mask;
                                false_mask = current.exclude(undefined_mask);
                            }
                        };

                        auto extract_null_undefined_equality = [&](Instruction* instruction) {
                            auto opcode = instruction->opcode();
                            bool is_equals = (opcode == Opcode::StrictlyEquals || opcode == Opcode::LooselyEquals);
                            bool is_inequals = (opcode == Opcode::StrictlyInequals || opcode == Opcode::LooselyInequals);
                            bool is_loose = (opcode == Opcode::LooselyEquals || opcode == Opcode::LooselyInequals);
                            if (!is_equals && !is_inequals)
                                return;

                            if (instruction->operand_count() < 2)
                                return;
                            auto* lhs = instruction->operand(0);
                            auto* rhs = instruction->operand(1);
                            if (!lhs || !rhs)
                                return;

                            Value* tested_value = nullptr;
                            bool is_null_or_undefined = false;

                            if (rhs->is_constant() && (rhs->constant_value().is_null() || rhs->constant_value().is_undefined())) {
                                tested_value = lhs;
                                is_null_or_undefined = true;
                            } else if (lhs->is_constant() && (lhs->constant_value().is_null() || lhs->constant_value().is_undefined())) {
                                tested_value = rhs;
                                is_null_or_undefined = true;
                            }

                            if (!tested_value || !is_null_or_undefined)
                                return;

                            // For loose equality, null == undefined is true, so both
                            // null and undefined match. For strict equality, only the
                            // exact type matches.
                            TypeMask literal_mask;
                            if (is_loose) {
                                literal_mask = TypeMask::nullish();
                            } else {
                                auto* constant_operand = (tested_value == lhs) ? rhs : lhs;
                                if (constant_operand->constant_value().is_null())
                                    literal_mask = TypeMask::for_type(Type::Null);
                                else
                                    literal_mask = TypeMask::for_type(Type::Undefined);
                            }

                            guarded_value = tested_value;
                            auto current = get_mask(tested_value);
                            if (is_equals) {
                                true_mask = current & literal_mask;
                                false_mask = current.exclude(literal_mask);
                            } else {
                                true_mask = current.exclude(literal_mask);
                                false_mask = current & literal_mask;
                            }
                        };

                        // Try each guard pattern.
                        extract_typeof_guard(condition_instruction);
                        if (!guarded_value)
                            extract_nullish_guard(condition_instruction);
                        if (!guarded_value)
                            extract_null_undefined_equality(condition_instruction);

                        // Install facts on true/false targets when recursing.
                        // NB: Only install facts on a target if it has a single
                        // predecessor (the branching block). If a target has
                        // multiple predecessors, the type fact from one edge
                        // doesn't hold for all paths to that block.
                        if (guarded_value) {
                            bool true_target_is_unique = true_target->predecessor_indices().size() == 1;
                            bool false_target_is_unique = false_target->predecessor_indices().size() == 1;

                            dominators.for_each_dominator_child(block, [&](BasicBlock& child) {
                                if (&child == true_target && true_target_is_unique && !true_mask.is_empty()) {
                                    set_fact(guarded_value->index(), true_mask);
                                    self(self, &child);
                                } else if (&child == false_target && false_target_is_unique && !false_mask.is_empty()) {
                                    set_fact(guarded_value->index(), false_mask);
                                    self(self, &child);
                                } else {
                                    self(self, &child);
                                }
                            });

                            // Restore scoped facts.
                            for (auto it = scoped_facts.rbegin(); it != scoped_facts.rend(); ++it)
                                type_facts.set(it->value, it->previous_mask);
                            return;
                        }
                    }
                }
            }
        }

        // No guard found -- recurse into dominator children without new facts.
        dominators.for_each_dominator_child(block, [&](BasicBlock& child) {
            self(self, &child);
        });

        // Restore scoped facts.
        for (auto it = scoped_facts.rbegin(); it != scoped_facts.rend(); ++it)
            type_facts.set(it->value, it->previous_mask);
    };

    process_block(process_block, function.entry_block());

    // Remove instructions that were marked dead during the walk.
    if (!dead_instructions.is_empty()) {
        for (auto& block : function.basic_blocks()) {
            block->remove_instructions_if([&](Instruction const& instruction) {
                return dead_instructions.contains(const_cast<Instruction*>(&instruction));
            });
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
