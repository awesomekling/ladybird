/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/QuickSort.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/SSAConstruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

SSAConstruction::SSAConstruction(Function& function, DominatorTree const& dominators, Bytecode::Executable const& executable,
    HashTable<u32> const& written_operands,
    HashMap<BasicBlock*, HashTable<u32>> const& block_actual_definitions,
    HashMap<BasicBlock*, HashMap<u32, Value*>>& block_definitions,
    HashMap<Value*, u32>& value_to_operand_raw)
    : m_function(function)
    , m_dominators(dominators)
    , m_executable(executable)
    , m_written_operands(written_operands)
    , m_block_actual_definitions(block_actual_definitions)
    , m_block_definitions(block_definitions)
    , m_value_to_operand_raw(value_to_operand_raw)
    , m_builder(function)
{
}

void SSAConstruction::run()
{
    place_phi_nodes();
    fill_phi_operands();
}

// Phase 1: Place phis at dominance frontiers of defining blocks
// This implements the standard SSA phi placement algorithm from Cytron et al.
void SSAConstruction::place_phi_nodes()
{
    // NB: Sort written operands for deterministic phi ordering across runs.
    //     HashTable iteration order depends on capacity, which varies between
    //     allocators (e.g. system malloc vs ASAN), causing different phi
    //     numbering in release vs sanitizer builds.
    Vector<u32> sorted_operands;
    sorted_operands.ensure_capacity(m_written_operands.size());
    for (auto raw : m_written_operands)
        sorted_operands.append(raw);
    quick_sort(sorted_operands);

    // For each written operand, compute where phis are needed
    for (auto raw : sorted_operands) {
        // Find all blocks that actually define this operand
        HashTable<BasicBlock*> def_blocks;
        for (auto& [block, defs] : m_block_actual_definitions) {
            if (defs.contains(raw))
                def_blocks.set(block);
        }

        // Compute iterated dominance frontier (where phis are needed)
        HashTable<BasicBlock*> phi_blocks;
        Vector<BasicBlock*> worklist;
        for (auto* block : def_blocks)
            worklist.append(block);

        while (!worklist.is_empty()) {
            auto* block = worklist.take_last();
            m_dominators.for_each_frontier_block(block, [&](BasicBlock& frontier_block) {
                if (!phi_blocks.contains(&frontier_block)) {
                    phi_blocks.set(&frontier_block);
                    // If this block doesn't already define the variable, add to worklist
                    // (the phi itself is a definition that extends the frontier)
                    if (!def_blocks.contains(&frontier_block))
                        worklist.append(&frontier_block);
                }
            });
        }

        // Place phis at the computed locations.
        // NB: Sort by block index for deterministic phi ordering across runs,
        //     since phi_blocks is a HashTable with pointer-based ordering.
        Vector<BasicBlock*> sorted_phi_blocks;
        sorted_phi_blocks.ensure_capacity(phi_blocks.size());
        for (auto* block : phi_blocks)
            sorted_phi_blocks.append(block);
        quick_sort(sorted_phi_blocks, [](auto* a, auto* b) { return a->index() < b->index(); });

        for (auto* block : sorted_phi_blocks) {
            auto const& preds = block->predecessors();
            if (preds.is_empty())
                continue;

            // Create an empty phi (we'll fill operands in phase 2)
            Vector<Value*> empty_values;
            Vector<BasicBlock*> empty_blocks;
            for (size_t i = 0; i < preds.size(); ++i) {
                empty_values.append(nullptr);
                empty_blocks.append(preds[i]);
            }

            m_builder.set_insertion_block(block);
            auto& phi = m_builder.build_phi(empty_values, empty_blocks);
            m_value_to_operand_raw.set(&phi, raw);

            // Update m_block_definitions to include the phi value, UNLESS the block
            // has an actual definition that would override it. This ensures successors
            // inherit the correct value.
            auto actual_defs = m_block_actual_definitions.get(block);
            if (!actual_defs.has_value() || !actual_defs->contains(raw)) {
                m_block_definitions.ensure(block).set(raw, &phi);
            }
        }
    }
}

// Phase 2: Fill in phi operands and rename uses using dominator tree walk
// This implements standard SSA renaming from Cytron et al.
void SSAConstruction::fill_phi_operands()
{
    // NB: We intentionally start with empty stacks. The standard SSA renaming
    // algorithm (Cytron et al.) builds up stacks during the dominator tree walk
    // by pushing definitions as they are encountered. Seeding stacks with
    // end-of-block definitions would cause early uses to be incorrectly rewritten
    // to later definitions within the same block.
    HashMap<u32, Vector<Value*>> operand_stacks;

    // Seed stacks with parameter values for all arguments.
    // Function parameters are implicit definitions at the entry block, but since
    // they aren't instruction results, the SSA renaming won't encounter them as
    // definitions. We must seed them so that the rename walk always has the
    // original parameter as the base reaching definition.
    for (auto* param : m_function.parameters()) {
        u32 raw = m_executable.argument_index_base + param->parameter_index();
        operand_stacks.ensure(raw).append(param);
    }

    // Walk dominator tree starting from entry block
    if (m_function.entry_block())
        rename_ssa(*m_function.entry_block(), operand_stacks);

    // Compute phi types by joining incoming value types.
    for (auto& block : m_function.basic_blocks()) {
        block->for_each_phi([&](PhiInstruction& phi) {
            auto const& operands = phi.operands();
            if (operands.is_empty())
                return;

            Type phi_type = Type::Unknown;
            bool first = true;

            for (auto* operand : operands) {
                if (!operand)
                    continue;

                if (first) {
                    phi_type = operand->type();
                    first = false;
                } else {
                    phi_type = join_types(phi_type, operand->type());
                }
            }

            if (phi_type != Type::Unknown)
                phi.result()->set_type(phi_type);

            // Re-derive result types for users whose types depend on operand
            // types, since the Phi's type may have widened after these
            // instructions were created (e.g. int32 -> number).
            for (auto* user : phi.result()->uses())
                user->recompute_result_type();
        });
    }
}

// Iterative SSA renaming using dominator tree walk
void SSAConstruction::rename_ssa(BasicBlock& start_block, HashMap<u32, Vector<Value*>>& stacks)
{
    struct WorkItem {
        BasicBlock* block;
        bool is_restore { false };
        HashMap<u32, size_t> entry_sizes;
    };

    Vector<WorkItem> work_stack;
    work_stack.empend(&start_block, false, HashMap<u32, size_t> {});

    while (!work_stack.is_empty()) {
        auto item = move(work_stack.last());
        work_stack.take_last();

        if (item.is_restore) {
            // Restore stack sizes (pop what we pushed in this block)
            for (auto& [op_raw, target_size] : item.entry_sizes) {
                auto& stack = stacks.ensure(op_raw);
                while (stack.size() > target_size)
                    stack.take_last();
            }
            continue;
        }

        auto& block = *item.block;

        // Record stack sizes at entry so we can restore them on exit
        HashMap<u32, size_t> entry_sizes;
        for (auto& [op_raw, stack] : stacks) {
            entry_sizes.set(op_raw, stack.size());
        }

        // Process phis first - they define values at block entry
        block.for_each_phi([&](PhiInstruction const& phi) {
            auto raw_opt = m_value_to_operand_raw.get(phi.result());
            if (raw_opt.has_value()) {
                stacks.ensure(*raw_opt).append(phi.result());
                if (!entry_sizes.contains(*raw_opt))
                    entry_sizes.set(*raw_opt, 0);
            }
        });

        // Rewrite operand uses in non-phi instructions and push new definitions
        for (auto& instruction : block.instructions()) {
            if (instruction->opcode() == Opcode::Phi)
                continue;

            // Rewrite operand uses to current stack top
            for (size_t i = 0; i < instruction->operands().size(); ++i) {
                auto* operand_value = instruction->operands()[i];
                if (!operand_value)
                    continue;

                auto raw_opt = m_value_to_operand_raw.get(operand_value);
                if (!raw_opt.has_value())
                    continue;

                auto stack_opt = stacks.get(*raw_opt);
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    auto* current = stack_opt->last();
                    if (current != operand_value)
                        instruction->set_operand(i, current);
                } else {
                    // No reaching definition: variable was never written on this path.
                    // Locals use the empty value (TDZ marker), registers use undefined.
                    auto decoded = m_executable.original_operand_from_raw(*raw_opt);
                    auto& default_value = m_function.create_constant(
                        decoded.is_local() ? js_special_empty_value() : js_undefined());
                    instruction->set_operand(i, &default_value);
                }
            }

            // If instruction defines a value, push it onto the stack
            if (instruction->result()) {
                auto raw_opt = m_value_to_operand_raw.get(instruction->result());
                if (raw_opt.has_value()) {
                    stacks.ensure(*raw_opt).append(instruction->result());
                    if (!entry_sizes.contains(*raw_opt))
                        entry_sizes.set(*raw_opt, 0);
                }
            }
        }

        // Fill phi operands in CFG successors
        auto fill_phi_for_successor = [&](BasicBlock* succ) {
            if (!succ)
                return;

            // Find our index in the successor's predecessor list
            size_t pred_index = SIZE_MAX;
            auto const& phi_preds = succ->predecessors();
            for (size_t i = 0; i < phi_preds.size(); ++i) {
                if (phi_preds[i] == &block) {
                    pred_index = i;
                    break;
                }
            }
            if (pred_index == SIZE_MAX)
                return;

            // Fill phi operands for this predecessor
            succ->for_each_phi([&](PhiInstruction& phi) {
                auto raw_opt = m_value_to_operand_raw.get(phi.result());
                if (!raw_opt.has_value())
                    return;

                // Get current value from stack
                auto stack_opt = stacks.get(*raw_opt);
                Value* reaching = nullptr;
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    reaching = stack_opt->last();
                } else {
                    // No definition reaches here.
                    // Locals use the empty value (TDZ marker), registers use undefined.
                    auto decoded = m_executable.original_operand_from_raw(*raw_opt);
                    reaching = &m_function.create_constant(
                        decoded.is_local() ? js_special_empty_value() : JS::js_undefined());
                }

                phi.set_incoming_value_for(block, reaching);
            });
        };

        // Fill phis for all CFG successors
        if (auto* term = block.terminator()) {
            fill_phi_for_successor(term->true_target());
            if (term->false_target() && term->false_target() != term->true_target())
                fill_phi_for_successor(term->false_target());
        }
        // Also fill phis for exception edges
        fill_phi_for_successor(block.exception_handler());
        if (block.finalizer() != block.exception_handler())
            fill_phi_for_successor(block.finalizer());

        // Push restore item (will be processed after all children)
        work_stack.empend(&block, true, move(entry_sizes));

        // Push dominated children in reverse order so first child is processed first
        Vector<BasicBlock*> children;
        m_dominators.for_each_dominator_child(&block, [&](BasicBlock& child) {
            children.append(&child);
        });
        for (int i = static_cast<int>(children.size()) - 1; i >= 0; --i) {
            work_stack.empend(children[i], false, HashMap<u32, size_t> {});
        }
    }
}

}
