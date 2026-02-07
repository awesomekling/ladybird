/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Queue.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/SimplifyCFG.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static inline size_t to_index(BlockIndex b) { return static_cast<u32>(b); }
static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

static size_t block_index_capacity(Function const& function)
{
    size_t max_index = 0;
    for (auto const& block : function.basic_blocks())
        max_index = max(max_index, to_index(block->index()) + 1);
    return max_index;
}

static void remove_dead_blocks(Function& function)
{
    HashTable<BasicBlock*> blocks_to_remove;
    for (auto& block : function.basic_blocks()) {
        if (block->instructions().is_empty())
            blocks_to_remove.set(block.ptr());
    }
    CFG::remove_blocks(function, blocks_to_remove);
}

static bool try_fold_constant_branches(Function& function)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        auto* term = block->terminator();
        if (!term || term->opcode() != Opcode::Branch)
            continue;

        // If both targets are the same, convert to unconditional jump
        if (term->true_target() == term->false_target() && term->true_target() != nullptr) {
            CFG::replace_branch_with_jump(*block, *term->true_target(), nullptr);
            changed = true;
            continue;
        }

        // Branch has condition as first operand
        if (term->operand_count() == 0)
            continue;

        auto* condition = term->operand(0);
        auto truthiness = condition->constant_truthiness();
        if (!truthiness.has_value())
            continue;

        bool take_true_branch = *truthiness;

        auto* target = take_true_branch ? term->true_target() : term->false_target();
        auto* not_taken = take_true_branch ? term->false_target() : term->true_target();

        // Replace Branch with Jump
        CFG::replace_branch_with_jump(*block, *target, not_taken);
        changed = true;
    }

    return changed;
}

static bool try_eliminate_empty_blocks(Function& function)
{
    bool changed = false;
    bool eliminated_any;

    do {
        eliminated_any = false;

        for (auto& block : function.basic_blocks()) {
            bool is_entry = block.ptr() == function.entry_block();

            // Check if block is empty (only a Jump instruction)
            if (block->instructions().size() != 1u)
                continue;

            auto* jump = block->terminator();
            if (!jump || jump->opcode() != Opcode::Jump)
                continue;

            auto* target = jump->true_target();
            if (!target)
                continue;

            // Don't eliminate if jumping to self
            if (target == block.ptr())
                continue;

            // Get predecessors of the empty block (resolve to pointers for iteration)
            Vector<BasicBlock*> predecessors;
            for (auto predecessor_index : block->predecessor_indices())
                predecessors.append(function.block_by_index(predecessor_index));

            // Don't eliminate if any predecessor would end up reaching the target
            // via two different paths with different phi values
            bool would_conflict = false;
            for (auto* pred : predecessors) {
                // Check if this predecessor already reaches the target directly
                // (via terminator edges or implicit exception handler/finalizer edges)
                auto* pred_term = pred->terminator();
                bool pred_already_reaches_target = (pred_term && (pred_term->true_target() == target || pred_term->false_target() == target))
                    || pred->exception_handler() == target
                    || pred->finalizer() == target;
                if (pred_already_reaches_target) {
                    // This predecessor can reach target both directly and via empty block
                    // Check if any phi in target would have different values for these paths
                    for (auto instruction_index : target->instructions()) {
                        auto* instruction = function.instruction_by_index(instruction_index);
                        if (instruction->opcode() != Opcode::Phi)
                            continue;

                        auto& phi = static_cast<PhiInstruction&>(*instruction);
                        auto* value_from_empty = phi.incoming_value_for(*block);
                        auto* value_from_direct = phi.incoming_value_for(*pred);

                        if (value_from_empty && value_from_direct && value_from_empty != value_from_direct) {
                            would_conflict = true;
                            break;
                        }
                    }
                }
                if (would_conflict)
                    break;
            }

            if (would_conflict)
                continue;

            // Entry block has no predecessors - only eliminate if target has no phis
            // and no other predecessors (otherwise it would become an entry block
            // with predecessors, violating the SSA invariant).
            if (is_entry) {
                bool can_eliminate = true;
                for (auto instruction_index : target->instructions()) {
                    if (function.instruction_by_index(instruction_index)->opcode() == Opcode::Phi) {
                        can_eliminate = false;
                        break;
                    }
                }
                // If the target has predecessors other than us, eliminating would
                // create an entry block with predecessors.
                for (auto pred_index : target->predecessor_indices()) {
                    if (pred_index != block->index()) {
                        can_eliminate = false;
                        break;
                    }
                }
                if (!can_eliminate)
                    continue;
            } else if (predecessors.is_empty()) {
                continue;
            }

            // If eliminating the entry block, make target the new entry
            if (is_entry)
                function.set_entry_block(target);

            // Redirect each predecessor edge from the empty block to the target
            for (auto* pred : predecessors) {
                CFG::redirect_edge(*pred, *block, *target, [&](Instruction& instr) -> Value* {
                    auto& target_phi = static_cast<PhiInstruction&>(instr);

                    // Find the value this phi expects from the empty block
                    auto* value_from_empty = target_phi.incoming_value_for(*block);
                    if (!value_from_empty)
                        return nullptr;

                    // If value_from_empty is a phi, trace to find what this pred would contribute
                    if (auto* def = value_from_empty->defining_instruction();
                        def && def->opcode() == Opcode::Phi) {
                        auto& def_phi = static_cast<PhiInstruction&>(*def);
                        if (auto* traced = def_phi.incoming_value_for(*pred))
                            return traced;
                    }

                    return value_from_empty;
                });
            }

            // Clear the block's instructions (will be removed later)
            block->clear_instructions();

            eliminated_any = true;
            changed = true;
            break; // Restart since we modified the CFG
        }
    } while (eliminated_any);

    return changed;
}

static bool try_merge_blocks(Function& function)
{
    bool changed = false;

    // Keep merging until no more opportunities
    bool merged_any;
    do {
        merged_any = false;

        for (auto& block_a : function.basic_blocks()) {
            auto* terminator = block_a->terminator();
            if (!terminator)
                continue;

            // Only merge if A ends with an unconditional jump
            if (terminator->opcode() != Opcode::Jump)
                continue;

            auto* block_b = terminator->true_target();
            if (!block_b)
                continue;

            // Don't merge self-loops
            if (block_b == block_a.ptr())
                continue;

            // B must have exactly one predecessor (A)
            if (block_b->predecessor_indices().size() != 1)
                continue;

            // B shouldn't be the entry block
            if (block_b == function.entry_block())
                continue;

            // Don't merge into blocks with phi nodes (preserves loop preheaders)
            if (!block_b->instructions().is_empty() && function.instruction_by_index(block_b->instructions().first())->opcode() == Opcode::Phi)
                continue;

            // Don't merge if exception handling context differs - this would change
            // which handler catches exceptions from B's instructions
            if (block_a->exception_handler() != block_b->exception_handler())
                continue;
            if (block_a->finalizer() != block_b->finalizer())
                continue;

            // Don't merge if the shared exception handler or finalizer has phi nodes.
            // Both A and B are EH predecessors of the handler, so merging would create
            // duplicate phi entries that can't be properly consolidated (A and B may
            // contribute different values to the handler's phis).
            auto handler_has_phis = [&](BasicBlock* target) {
                return target && !target->instructions().is_empty()
                    && function.instruction_by_index(target->instructions().first())->opcode() == Opcode::Phi;
            };
            if (handler_has_phis(block_a->exception_handler()) || handler_has_phis(block_a->finalizer()))
                continue;

            // Merge B into A:
            // 1. Capture B's successors before modifying anything
            auto* b_terminator = block_b->terminator();
            BasicBlock* b_true_target = b_terminator ? b_terminator->true_target() : nullptr;
            BasicBlock* b_false_target = b_terminator ? b_terminator->false_target() : nullptr;

            // 2. Remove the Jump from A
            block_a->remove_terminator();

            // 3. Move all instructions from B to A
            for (auto instruction_index : block_b->take_all_instructions())
                block_a->append_instruction(instruction_index);

            // 4. Update all references to B to point to A
            CFG::retarget_all_edges(function, *block_b, *block_a);

            // 5. Add A to the predecessor lists of B's former successors
            if (b_true_target && b_true_target != block_b)
                CFG::add_predecessor(*b_true_target, *block_a);
            if (b_false_target && b_false_target != block_b && b_false_target != b_true_target)
                CFG::add_predecessor(*b_false_target, *block_a);

            merged_any = true;
            changed = true;
            break; // Restart iteration since we modified the list
        }
    } while (merged_any);

    return changed;
}

static bool try_thread_jumps(Function& function, DominatorTree& dominators)
{
    bool changed = false;

    // Look for blocks where a Branch condition is a Phi node
    // and some phi inputs are constants
    for (auto& block : function.basic_blocks()) {
        auto* terminator = block->terminator();
        if (!terminator || terminator->opcode() != Opcode::Branch)
            continue;

        if (terminator->operand_count() == 0)
            continue;

        auto* condition = terminator->operand(0);
        if (!condition->defining_instruction())
            continue;

        auto* phi_instr = condition->defining_instruction();
        if (phi_instr->opcode() != Opcode::Phi)
            continue;

        auto& phi = static_cast<PhiInstruction&>(*phi_instr);

        // The phi must be in this same block
        if (phi.parent_block() != block.ptr())
            continue;

        auto* true_target = terminator->true_target();
        auto* false_target = terminator->false_target();

        if (!true_target || !false_target)
            continue;

        // For each phi predecessor with a constant value, we can thread
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            auto* pred_block = function.block_by_index(phi.incoming_block(i));
            auto* pred_value = phi.incoming_value(i);

            if (!pred_value)
                continue;

            auto truthiness = pred_value->constant_truthiness();
            if (!truthiness.has_value())
                continue;

            bool take_true = *truthiness;

            auto* thread_target = take_true ? true_target : false_target;

            // Check if the bypassed block has side effects that must execute.
            bool bypassed_block_has_side_effects = false;
            block->for_each_instruction([&](Instruction const& instruction) {
                if (instruction.opcode() == Opcode::Phi)
                    return;
                if (instruction.is_terminator())
                    return;
                if (instruction.has_side_effects())
                    bypassed_block_has_side_effects = true;
            });

            if (bypassed_block_has_side_effects)
                continue;

            // Check if thread_target uses any values defined in the bypassed block
            // (except through phi nodes in thread_target that we'll update)
            bool target_uses_bypassed_values = false;
            thread_target->for_each_instruction([&](Instruction const& instruction) {
                if (instruction.opcode() == Opcode::Phi)
                    return;
                for (size_t oi = 0; oi < instruction.operand_count(); ++oi) {
                    auto* operand = instruction.operand(oi);
                    if (operand->defining_instruction() && operand->defining_instruction()->parent_block() == block.ptr())
                        target_uses_bypassed_values = true;
                }
            });

            if (target_uses_bypassed_values)
                continue;

            // Check if the phi result is used outside the bypassed block.
            bool phi_used_outside_block = false;
            if (auto* phi_result = phi.result()) {
                for (auto const& use : phi_result->uses()) {
                    if (function.instruction_by_index(use.instruction) != terminator) {
                        phi_used_outside_block = true;
                        break;
                    }
                }
            }

            if (phi_used_outside_block)
                continue;

            // The bypassed block must dominate pred_block.
            if (!dominators.dominates(block.ptr(), pred_block))
                continue;

            if (!pred_block->terminator())
                continue;

            // Check if pred actually targets this block
            auto* pred_terminator = pred_block->terminator();
            if (pred_terminator->true_target() != block.ptr() && pred_terminator->false_target() != block.ptr())
                continue;

            // Precompute traced phi values before redirect_edge removes pred_block
            // from the bypassed block's phis (which would break tracing).
            HashMap<Instruction*, Value*> traced_values;
            thread_target->for_each_phi([&](PhiInstruction& target_phi) {
                auto* value_from_bypassed = target_phi.incoming_value_for(*block);

                if (!value_from_bypassed)
                    return;

                // If value_from_bypassed is a phi in the bypassed block, trace to find
                // what value pred_block would contribute
                if (auto* def = value_from_bypassed->defining_instruction();
                    def && def->opcode() == Opcode::Phi && def->parent_block() == block.ptr()) {
                    auto& def_phi = static_cast<PhiInstruction&>(*def);
                    if (auto* traced = def_phi.incoming_value_for(*pred_block))
                        traced_values.set(&target_phi, traced);
                } else {
                    traced_values.set(&target_phi, value_from_bypassed);
                }
            });

            CFG::redirect_edge(*pred_block, *block, *thread_target, [&](Instruction& instr) -> Value* {
                if (auto it = traced_values.find(&instr); it != traced_values.end())
                    return it->value;
                return nullptr;
            });

            changed = true;
        }
    }

    return changed;
}

static bool try_eliminate_unreachable_blocks(Function& function)
{
    auto capacity = block_index_capacity(function);

    // Find all reachable blocks using BFS from entry
    auto reachable = MUST(AK::Bitmap::create(capacity, false));
    Queue<BasicBlock*> worklist;

    if (auto* entry = function.entry_block()) {
        worklist.enqueue(entry);
        reachable.set(to_index(entry->index()), true);
    }

    while (!worklist.is_empty()) {
        auto* block = worklist.dequeue();

        CFG::for_each_successor(*block, [&](BasicBlock& target) {
            auto ti = to_index(target.index());
            if (!reachable.get(ti)) {
                reachable.set(ti, true);
                worklist.enqueue(&target);
            }
        });
    }

    // Collect dead blocks
    Vector<BasicBlock*> dead_blocks;
    for (auto& block : function.basic_blocks()) {
        if (!reachable.get(to_index(block->index())))
            dead_blocks.append(block.ptr());
    }

    if (dead_blocks.is_empty())
        return false;

    // Build a map of phi results from dead blocks to their replacement values
    // If a phi in a dead block has all identical operands, we can replace uses of
    // the phi result with that operand value
    auto value_capacity = function.values().size();
    Vector<Optional<ValueIndex>> dead_phi_replacements;
    dead_phi_replacements.resize(value_capacity);

    for (auto* dead_block : dead_blocks) {
        dead_block->for_each_phi([&](PhiInstruction const& phi) {
            if (phi.operand_count() == 0)
                return;

            // Check if all operands are the same
            Value* replacement = phi.operand(0);
            bool all_same = true;
            for (size_t i = 1; i < phi.operand_count(); ++i) {
                if (phi.operand(i) != replacement) {
                    all_same = false;
                    break;
                }
            }

            if (all_same && replacement && phi.result())
                dead_phi_replacements[to_index(phi.result()->index())] = replacement->index();
        });
    }

    // Replace uses of dead phi results in live blocks
    for (auto& block : function.basic_blocks()) {
        if (!reachable.get(to_index(block->index())))
            continue;

        block->for_each_instruction([&](Instruction& instruction) {
            for (size_t i = 0; i < instruction.operand_count(); ++i) {
                auto* operand = instruction.operand(i);
                if (!operand)
                    continue;

                // Follow the replacement chain
                auto vi = to_index(operand->index());
                while (vi < dead_phi_replacements.size() && dead_phi_replacements[vi].has_value())
                    vi = to_index(*dead_phi_replacements[vi]);

                if (vi != to_index(operand->index()))
                    instruction.set_operand(i, function.values()[vi].ptr());
            }
        });
    }

    // Remove dead blocks and clean up all references to them.
    HashTable<BasicBlock*> dead_set;
    for (auto* block : dead_blocks)
        dead_set.set(block);
    CFG::remove_blocks(function, dead_set);

    return true;
}

PreservedAnalyses SimplifyCFG::run(Function& function, PassManager&)
{
    bool changed = false;

    bool iteration_changed;
    do {
        iteration_changed = false;
        iteration_changed |= try_fold_constant_branches(function);
        DominatorTree dominators(function);
        iteration_changed |= try_thread_jumps(function, dominators);
        iteration_changed |= try_eliminate_empty_blocks(function);
        iteration_changed |= try_merge_blocks(function);
        changed |= iteration_changed;
    } while (iteration_changed);

    // BFS reachability prune runs once after the fixpoint
    changed |= try_eliminate_unreachable_blocks(function);

    remove_dead_blocks(function);

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
