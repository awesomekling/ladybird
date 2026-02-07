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
#include <LibJS/IR/Passes/RedundantCheckElimination.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Bitmask of guard opcodes verified for an SSA value.
enum GuardBit : u8 {
    NotObject = 1 << 0,
    Nullish = 1 << 1,
    TDZ = 1 << 2,
};

static constexpr TypeMask object_types()
{
    return TypeMask::for_type(Type::Object)
        | TypeMask::for_type(Type::Function)
        | TypeMask::for_type(Type::Array);
}

PreservedAnalyses RedundantCheckElimination::run(Function& function, PassManager& pass_manager)
{
    if (!function.entry_block())
        return PreservedAnalyses::all();

    bool changed = false;
    auto const& dominators = pass_manager.dominator_tree(function);
    HashTable<Instruction*> dead_instructions;

    // Map from SSA value to bitmask of guards already verified.
    HashMap<ValueIndex, u8> guard_facts;

    struct ScopedEntry {
        ValueIndex value;
        u8 previous_bits;
    };

    auto get_type_mask = [&](Value* value) -> TypeMask {
        if (!value)
            return TypeMask::all();
        if (value->type() != Type::Unknown)
            return TypeMask::for_type(value->type());
        return TypeMask::all();
    };

    auto process_block = [&](auto& self, BasicBlock* block) -> void {
        Vector<ScopedEntry> scoped_entries;

        auto set_guard = [&](ValueIndex index, u8 bits) {
            auto it = guard_facts.find(index);
            u8 previous = (it != guard_facts.end()) ? it->value : 0;
            scoped_entries.append({ index, previous });
            guard_facts.set(index, previous | bits);
        };

        block->for_each_instruction([&](Instruction& instruction) {
            switch (instruction.opcode()) {

            case Opcode::ThrowIfNotObject: {
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto* operand = instruction.operand(0);
                auto index = operand->index();

                // Check if already guarded by a dominating ThrowIfNotObject.
                auto it = guard_facts.find(index);
                u8 existing = (it != guard_facts.end()) ? it->value : 0;
                if (existing & GuardBit::NotObject) {
                    dead_instructions.set(&instruction);
                    changed = true;
                    break;
                }

                // Check if the operand's type guarantees it's an object.
                auto mask = get_type_mask(operand);
                if (mask != TypeMask::all() && (mask & ~object_types()).is_empty()) {
                    dead_instructions.set(&instruction);
                    changed = true;
                    break;
                }

                // Record the guard. ThrowIfNotObject implies ThrowIfNullish
                // (objects are never nullish).
                set_guard(index, GuardBit::NotObject | GuardBit::Nullish);
                break;
            }

            case Opcode::ThrowIfNullish: {
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto* operand = instruction.operand(0);
                auto index = operand->index();

                // Check if already guarded.
                auto it = guard_facts.find(index);
                u8 existing = (it != guard_facts.end()) ? it->value : 0;
                if (existing & GuardBit::Nullish) {
                    dead_instructions.set(&instruction);
                    changed = true;
                    break;
                }

                // Check if the operand's type excludes nullish.
                auto mask = get_type_mask(operand);
                if (mask != TypeMask::all() && mask.excludes_nullish()) {
                    dead_instructions.set(&instruction);
                    changed = true;
                    break;
                }

                set_guard(index, GuardBit::Nullish);
                break;
            }

            case Opcode::ThrowIfTDZ: {
                if (instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto* operand = instruction.operand(0);
                auto index = operand->index();

                // Check if already guarded by a dominating ThrowIfTDZ.
                auto it = guard_facts.find(index);
                u8 existing = (it != guard_facts.end()) ? it->value : 0;
                if (existing & GuardBit::TDZ) {
                    dead_instructions.set(&instruction);
                    changed = true;
                    break;
                }

                set_guard(index, GuardBit::TDZ);
                break;
            }

            // Identity conversion elimination.
            case Opcode::ToNumber: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto type = instruction.operand(0)->type();
                if (type == Type::Number || type == Type::Int32) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            case Opcode::ToInt32: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                if (instruction.operand(0)->type() == Type::Int32) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            case Opcode::ToBoolean: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                if (instruction.operand(0)->type() == Type::Boolean) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            case Opcode::ToString: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                if (instruction.operand(0)->type() == Type::String) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            case Opcode::ToObject: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto type = instruction.operand(0)->type();
                if (type == Type::Object || type == Type::Function || type == Type::Array) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            case Opcode::ToNumeric: {
                if (!instruction.result() || instruction.operand_count() == 0 || !instruction.operand(0))
                    break;
                auto type = instruction.operand(0)->type();
                if (type == Type::Number || type == Type::Int32 || type == Type::BigInt) {
                    instruction.result()->replace_all_uses_with(instruction.operand(0));
                    dead_instructions.set(&instruction);
                    changed = true;
                }
                break;
            }

            default:
                break;
            }
        });

        // Recurse into dominator tree children.
        dominators.for_each_dominator_child(block, [&](BasicBlock& child) {
            self(self, &child);
        });

        // Restore scoped guard facts.
        for (auto it = scoped_entries.rbegin(); it != scoped_entries.rend(); ++it) {
            if (it->previous_bits == 0)
                guard_facts.remove(it->value);
            else
                guard_facts.set(it->value, it->previous_bits);
        }
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
