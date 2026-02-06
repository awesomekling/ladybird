/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <AK/QuickSort.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/SSAConstruction.h>
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

static Vector<BasicBlock*> build_block_index_table(Function const& function, size_t capacity)
{
    Vector<BasicBlock*> table;
    table.resize_with_default_value(capacity, nullptr);
    for (auto const& block : function.basic_blocks())
        table[to_index(block->index())] = block.ptr();
    return table;
}

SSAConstruction::SSAConstruction(Function& function, DominatorTree const& dominators, Bytecode::Executable const& executable,
    HashTable<u32> const& written_operands,
    Vector<HashTable<u32>> const& block_actual_definitions,
    Vector<HashMap<u32, Value*>>& block_definitions,
    Vector<Optional<u32>>& value_to_operand_raw)
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

    auto capacity = block_index_capacity(m_function);
    auto block_table = build_block_index_table(m_function, capacity);

    // For each written operand, compute where phis are needed
    for (auto raw : sorted_operands) {
        // Find all blocks that actually define this operand
        auto def_blocks = MUST(AK::Bitmap::create(capacity, false));
        for (auto const& block : m_function.basic_blocks()) {
            auto bi = to_index(block->index());
            if (bi < m_block_actual_definitions.size() && m_block_actual_definitions[bi].contains(raw))
                def_blocks.set(bi, true);
        }

        // Compute iterated dominance frontier (where phis are needed)
        auto phi_blocks = MUST(AK::Bitmap::create(capacity, false));
        Vector<BlockIndex> worklist;
        for (size_t i = 0; i < capacity; ++i) {
            if (def_blocks.get(i))
                worklist.append(static_cast<BlockIndex>(i));
        }

        while (!worklist.is_empty()) {
            auto block_idx = worklist.take_last();
            auto* block = block_table[to_index(block_idx)];
            if (!block)
                continue;
            m_dominators.for_each_frontier_block(block, [&](BasicBlock& frontier_block) {
                auto fi = to_index(frontier_block.index());
                if (!phi_blocks.get(fi)) {
                    phi_blocks.set(fi, true);
                    // If this block doesn't already define the variable, add to worklist
                    // (the phi itself is a definition that extends the frontier)
                    if (!def_blocks.get(fi))
                        worklist.append(frontier_block.index());
                }
            });
        }

        // Place phis at the computed locations.
        // NB: Bitmap iteration is inherently ordered by index, ensuring
        //     deterministic phi ordering across runs.
        for (size_t bi = 0; bi < capacity; ++bi) {
            if (!phi_blocks.get(bi))
                continue;
            auto* block = block_table[bi];
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

            auto vi = to_index(phi.index());
            if (vi >= m_value_to_operand_raw.size())
                m_value_to_operand_raw.resize(vi + 1);
            m_value_to_operand_raw[vi] = raw;

            // Update m_block_definitions to include the phi value, UNLESS the block
            // has an actual definition that would override it. This ensures successors
            // inherit the correct value.
            bool has_actual_def = bi < m_block_actual_definitions.size() && m_block_actual_definitions[bi].contains(raw);
            if (!has_actual_def) {
                if (bi >= m_block_definitions.size())
                    m_block_definitions.resize(bi + 1);
                m_block_definitions[bi].set(raw, &phi);
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
            auto vi = to_index(phi.result()->index());
            if (vi < m_value_to_operand_raw.size() && m_value_to_operand_raw[vi].has_value()) {
                auto raw = *m_value_to_operand_raw[vi];
                stacks.ensure(raw).append(phi.result());
                if (!entry_sizes.contains(raw))
                    entry_sizes.set(raw, 0);
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

                auto vi = to_index(operand_value->index());
                if (vi >= m_value_to_operand_raw.size() || !m_value_to_operand_raw[vi].has_value())
                    continue;
                auto raw = *m_value_to_operand_raw[vi];

                auto stack_opt = stacks.get(raw);
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    auto* current = stack_opt->last();
                    if (current != operand_value)
                        instruction->set_operand(i, current);
                } else {
                    // No reaching definition: variable was never written on this path.
                    // Locals use the empty value (TDZ marker), registers use undefined.
                    auto decoded = m_executable.original_operand_from_raw(raw);
                    auto& default_value = m_function.create_constant(
                        decoded.is_local() ? js_special_empty_value() : js_undefined());
                    instruction->set_operand(i, &default_value);
                }
            }

            // If instruction defines a value, push it onto the stack
            if (instruction->result()) {
                auto vi = to_index(instruction->result()->index());
                if (vi < m_value_to_operand_raw.size() && m_value_to_operand_raw[vi].has_value()) {
                    auto raw = *m_value_to_operand_raw[vi];
                    stacks.ensure(raw).append(instruction->result());
                    if (!entry_sizes.contains(raw))
                        entry_sizes.set(raw, 0);
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
                auto vi = to_index(phi.result()->index());
                if (vi >= m_value_to_operand_raw.size() || !m_value_to_operand_raw[vi].has_value())
                    return;
                auto raw = *m_value_to_operand_raw[vi];

                // Get current value from stack
                auto stack_opt = stacks.get(raw);
                Value* reaching = nullptr;
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    reaching = stack_opt->last();
                } else {
                    // No definition reaches here.
                    // Locals use the empty value (TDZ marker), registers use undefined.
                    auto decoded = m_executable.original_operand_from_raw(raw);
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
