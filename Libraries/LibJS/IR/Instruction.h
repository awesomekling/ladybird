/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/Bytecode/Builtins.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/IdentifierTable.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/PropertyKeyTable.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/OpcodeList.h>
#include <LibJS/IR/Type.h>
#include <LibJS/Runtime/Completion.h>
#include <LibJS/Runtime/Iterator.h>

namespace JS {

class ClassExpression;
class FunctionNode;

}

namespace JS::IR {

// Effect bitmask describing what observable effects an instruction may have.
// Used by passes (DCE, GVN, LICM, SimplifyCFG) to reason about instruction safety.
enum class Effects : u8 {
    None = 0,
    MayThrow = 1 << 0,    // May throw an exception
    ReadsState = 1 << 1,  // Reads mutable state (heap, environment)
    WritesState = 1 << 2, // Writes mutable state
    Calls = 1 << 3,       // May invoke arbitrary user code (subsumes all others)
    Allocates = 1 << 4,   // Allocates a new heap object (not pure, but removable if unused)
};
AK_ENUM_BITWISE_OPERATORS(Effects)

#define IR_OPCODE_ENUM(name, ...) name,
enum class [[clang::enum_extensibility(closed)]] Opcode : u8 {
    IR_OPCODE_LIST(IR_OPCODE_ENUM)
        __Count, // Sentinel for static assertions - must be last
};
#undef IR_OPCODE_ENUM

// Whether an opcode's effects can be refined based on operand types.
enum class EffectRefinement : u8 {
    None,                 // No refinement possible
    PureOnSafePrimitives, // Becomes pure when all operands are safe primitives
};

// Centralized opcode metadata table.
// All opcode properties are defined here to prevent divergence between switches.
struct OpcodeTraits {
    char const* name;
    bool is_terminator;
    Effects effects; // Conservative effect bitmask (refined by Instruction::effects())
    bool is_call_like;
    bool has_result;
    u8 operand_arity;            // Expected operand count (VariableArity = variable)
    Type guaranteed_result_type; // Static result type (Type::Unknown = no guarantee)
    bool is_commutative;         // Commutative binary op (operand order doesn't matter)
    u8 tuple_arity;              // Number of tuple elements produced (0 = not a tuple)
    EffectRefinement refinement; // Whether effects can be refined based on operand types
};

static constexpr u8 VariableArity = 255;

// Shorthand aliases for Effects combinations used in the opcode list.
static constexpr auto E_NONE = Effects::None;
static constexpr auto E_THROW = Effects::MayThrow;
static constexpr auto E_READ = Effects::ReadsState;
static constexpr auto E_WRITE = Effects::WritesState;
static constexpr auto E_CALL = Effects::Calls;
static constexpr auto E_ALLOC = Effects::Allocates;
static constexpr auto E_THROW_WRITE = Effects::MayThrow | Effects::WritesState;

// Shorthand aliases for EffectRefinement used in the opcode list.
static constexpr auto R_NONE = EffectRefinement::None;
static constexpr auto R_PRIM = EffectRefinement::PureOnSafePrimitives;

// clang-format off
#define IR_OPCODE_TRAITS(name, is_term, eff, call, result, arity, type, comm, tuple, refine) \
    { #name, is_term, eff, call, result, arity, type, comm, tuple, refine },
static constexpr OpcodeTraits s_opcode_traits[] = {
    IR_OPCODE_LIST(IR_OPCODE_TRAITS)
};
#undef IR_OPCODE_TRAITS
// clang-format on

static_assert(AK::array_size(s_opcode_traits) == to_underlying(Opcode::__Count),
    "OpcodeTraits table must have exactly one entry per opcode");

constexpr OpcodeTraits const& opcode_traits(Opcode opcode)
{
    VERIFY(opcode != Opcode::__Count);
    return s_opcode_traits[to_underlying(opcode)];
}

constexpr bool is_terminator_opcode(Opcode opcode)
{
    return opcode_traits(opcode).is_terminator;
}

// Conservative opcode-level effects (no type refinement).
constexpr Effects opcode_effects(Opcode opcode)
{
    return opcode_traits(opcode).effects;
}

// Derived opcode-level queries from effects.
constexpr bool may_throw_opcode(Opcode opcode)
{
    return has_any_flag(opcode_effects(opcode), Effects::MayThrow | Effects::Calls);
}

constexpr char const* opcode_to_string(Opcode opcode)
{
    return opcode_traits(opcode).name;
}

// Does this opcode produce a result value?
constexpr bool opcode_has_result(Opcode opcode)
{
    return opcode_traits(opcode).has_result;
}

// Expected operand count for this opcode (VariableArity = variable).
constexpr u8 opcode_operand_arity(Opcode opcode)
{
    return opcode_traits(opcode).operand_arity;
}

constexpr bool has_variable_arity(Opcode opcode)
{
    return opcode_operand_arity(opcode) == VariableArity;
}

// Static result type guaranteed by this opcode (Type::Unknown = no guarantee).
constexpr Type opcode_guaranteed_result_type(Opcode opcode)
{
    return opcode_traits(opcode).guaranteed_result_type;
}

// Is this a commutative binary op (operand order doesn't matter)?
constexpr bool is_commutative_opcode(Opcode opcode)
{
    return opcode_traits(opcode).is_commutative;
}

// Number of tuple elements produced by this opcode (0 = not a tuple).
constexpr u8 opcode_tuple_arity(Opcode opcode)
{
    return opcode_traits(opcode).tuple_arity;
}

// Returns the inverted comparison opcode, if the opcode is a comparison.
constexpr Optional<Opcode> inverted_comparison_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::StrictlyEquals:
        return Opcode::StrictlyInequals;
    case Opcode::StrictlyInequals:
        return Opcode::StrictlyEquals;
    case Opcode::LooselyEquals:
        return Opcode::LooselyInequals;
    case Opcode::LooselyInequals:
        return Opcode::LooselyEquals;
    case Opcode::LessThan:
        return Opcode::GreaterThanEquals;
    case Opcode::LessThanEquals:
        return Opcode::GreaterThan;
    case Opcode::GreaterThan:
        return Opcode::LessThanEquals;
    case Opcode::GreaterThanEquals:
        return Opcode::LessThan;
    default:
        return {};
    }
}

// Does this opcode require a specialized instruction class?
// These opcodes must NOT use the generic Instruction::create<Op>().
constexpr bool requires_specialized_instruction(Opcode opcode)
{
    switch (opcode) {
    // Use JumpInstruction::create() or BranchInstruction::create()
    case Opcode::Jump:
    case Opcode::ContinuePendingUnwind:
    case Opcode::EnterUnwindContext:
    case Opcode::Branch:
    // Use PhiInstruction::create()
    case Opcode::Phi:
    // Use ParallelCopyInstruction::create()
    case Opcode::ParallelCopy:
    // Use GetByIdInstruction::create()
    case Opcode::GetById:
    // Use CallInstruction::create()
    case Opcode::Call:
    case Opcode::CallBuiltin:
    case Opcode::CallDirectEval:
    case Opcode::CallWithArgumentArray:
        return true;
    default:
        return false;
    }
}

class TerminatorInstruction;

class JS_API Instruction {
    AK_MAKE_NONCOPYABLE(Instruction);
    AK_MAKE_NONMOVABLE(Instruction);

public:
    template<Opcode Op>
    [[nodiscard]] static NonnullOwnPtr<Instruction> create()
    {
        static_assert(!requires_specialized_instruction(Op),
            "This opcode requires a specialized instruction class. "
            "Use JumpInstruction, BranchInstruction, PhiInstruction, "
            "GetByIdInstruction, or CallInstruction instead.");
        static_assert(!is_terminator_opcode(Op),
            "Terminator opcodes require TerminatorInstruction::create<Op>().");
        return adopt_own(*new Instruction(Op));
    }

    virtual ~Instruction() = default;

    Opcode opcode() const { return m_opcode; }

    // Safe opcode mutation for comparison inversion.
    // Returns true if the opcode was changed, false if not a comparison.
    // This is the only safe way to mutate an opcode in-place.
    bool try_invert_comparison();

    InstructionIndex index() const { return m_index; }
    void set_index(InstructionIndex index) { m_index = index; }

    BasicBlock* parent_block() const { return m_parent_block; }
    void set_parent_block(BasicBlock* block) { m_parent_block = block; }

    Function* parent_function() const { return m_function; }
    void set_parent_function(Function* function) { m_function = function; }

    Value* result() const;
    Optional<ValueIndex> result_index() const { return m_result; }
    void set_result(Value* value);

    size_t operand_count() const { return m_operands.size(); }
    Value* operand(size_t index) const;
    Optional<ValueIndex> operand_index(size_t index) const { return m_operands[index]; }
    void add_operand(Value* value);
    void add_operand_uses();
    void set_operand(size_t index, Value* value);
    void clear_operand_uses();

    // For Phi nodes
    Vector<BlockIndex> const& phi_predecessors() const { return m_phi_predecessors; }
    void add_phi_operand(BlockIndex predecessor, Value* value);
    void set_phi_predecessor(size_t index, BlockIndex block) { m_phi_predecessors[index] = block; }
    void remove_phi_operand(size_t index);

    // Instruction-specific indices (reuse bytecode tables)
    Bytecode::PropertyKeyTableIndex property_key_index() const { return m_property_key_index; }
    void set_property_key_index(Bytecode::PropertyKeyTableIndex index) { m_property_key_index = index; }

    Bytecode::IdentifierTableIndex identifier_index() const { return m_identifier_index; }
    void set_identifier_index(Bytecode::IdentifierTableIndex index) { m_identifier_index = index; }

    CacheIndex cache_index() const { return m_cache_index; }
    void set_cache_index(CacheIndex index) { m_cache_index = index; }

    PropertySlot property_slot() const { return m_property_slot; }
    void set_property_slot(PropertySlot slot) { m_property_slot = slot; }

    // For NewFunction - reference to the AST node
    FunctionNode const* function_node() const { return m_function_node; }
    void set_function_node(FunctionNode const* node) { m_function_node = node; }
    Optional<Bytecode::IdentifierTableIndex> lhs_name() const { return m_lhs_name; }
    void set_lhs_name(Optional<Bytecode::IdentifierTableIndex> name) { m_lhs_name = name; }
    Optional<Bytecode::IdentifierTableIndex> base_identifier() const { return m_base_identifier; }
    void set_base_identifier(Optional<Bytecode::IdentifierTableIndex> id) { m_base_identifier = id; }

    // For NewClass - reference to the AST node
    ClassExpression const* class_expression() const { return m_class_expression; }
    void set_class_expression(ClassExpression const* node) { m_class_expression = node; }

    // For NewRegExp
    Bytecode::StringTableIndex regex_source_index() const { return m_regex_source_index; }
    void set_regex_source_index(Bytecode::StringTableIndex index) { m_regex_source_index = index; }
    Bytecode::StringTableIndex regex_flags_index() const { return m_regex_flags_index; }
    void set_regex_flags_index(Bytecode::StringTableIndex index) { m_regex_flags_index = index; }
    Bytecode::RegexTableIndex regex_index() const { return m_regex_index; }
    void set_regex_index(Bytecode::RegexTableIndex index) { m_regex_index = index; }

    // For ExtractValue - which element to extract from a tuple
    u32 extract_index() const { return m_extract_index; }
    void set_extract_index(u32 index) { m_extract_index = index; }

    // For GetIterator - sync or async
    IteratorHint iterator_hint() const { return m_iterator_hint; }
    void set_iterator_hint(IteratorHint hint) { m_iterator_hint = hint; }

    // For CreateVariable
    Bytecode::Op::EnvironmentMode environment_mode() const { return m_environment_mode; }
    void set_environment_mode(Bytecode::Op::EnvironmentMode mode) { m_environment_mode = mode; }
    bool is_immutable() const { return m_is_immutable; }
    void set_is_immutable(bool value) { m_is_immutable = value; }
    bool is_global() const { return m_is_global; }
    void set_is_global(bool value) { m_is_global = value; }
    bool is_strict() const { return m_is_strict; }
    void set_is_strict(bool value) { m_is_strict = value; }

    // For CreateLexicalEnvironment
    u32 capacity() const { return m_capacity; }
    void set_capacity(u32 capacity) { m_capacity = capacity; }

    // For CallDirectEval and other calls
    Optional<Bytecode::StringTableIndex> expression_string() const { return m_expression_string; }
    void set_expression_string(Optional<Bytecode::StringTableIndex> index) { m_expression_string = index; }

    // For CallBuiltin
    Bytecode::Builtin builtin() const { return m_builtin; }
    void set_builtin(Bytecode::Builtin builtin) { m_builtin = builtin; }

    // For CreateArguments
    Bytecode::Op::ArgumentsKind arguments_kind() const { return m_arguments_kind; }
    void set_arguments_kind(Bytecode::Op::ArgumentsKind kind) { m_arguments_kind = kind; }
    bool create_arguments_needs_dst() const { return m_create_arguments_needs_dst; }
    void set_create_arguments_needs_dst(bool value) { m_create_arguments_needs_dst = value; }

    // For CreateRestParams
    u32 rest_index() const { return m_rest_index; }
    void set_rest_index(u32 index) { m_rest_index = index; }

    // For SuperCallWithArgumentArray
    bool is_synthetic() const { return m_is_synthetic; }
    void set_is_synthetic(bool value) { m_is_synthetic = value; }

    // For ArrayAppend
    bool is_spread() const { return m_is_spread; }
    void set_is_spread(bool value) { m_is_spread = value; }

    // For SetCompletionType / AsyncIteratorClose
    Completion::Type completion_type() const { return m_completion_type; }
    void set_completion_type(Completion::Type type) { m_completion_type = type; }

    // For NewTypeError
    Bytecode::StringTableIndex string_table_index() const { return m_string_table_index; }
    void set_string_table_index(Bytecode::StringTableIndex index) { m_string_table_index = index; }

    // Source location tracking
    Optional<Bytecode::SourceRecord> const& source_record() const { return m_source_record; }
    void set_source_record(Bytecode::SourceRecord record) { m_source_record = record; }

    bool is_terminator() const { return is_terminator_opcode(m_opcode); }

    // Re-derive result type from current operand types.
    // Called during initial construction and after Phi type resolution.
    void recompute_result_type();

    // Type-aware effect bitmask (refines opcode-level effects using operand types).
    Effects effects() const;

    // Derived queries from effects().
    bool has_side_effects() const;
    bool is_pure() const;
    bool is_hoistable() const;
    bool may_throw() const;

protected:
    explicit Instruction(Opcode opcode);

private:
    Opcode m_opcode;
    InstructionIndex m_index { 0 };
    BasicBlock* m_parent_block { nullptr };
    Function* m_function { nullptr };
    Optional<ValueIndex> m_result;
    Vector<Optional<ValueIndex>> m_operands;

    // For Phi
    Vector<BlockIndex> m_phi_predecessors;

    // Instruction-specific indices
    Bytecode::PropertyKeyTableIndex m_property_key_index;
    Bytecode::IdentifierTableIndex m_identifier_index;
    CacheIndex m_cache_index { 0 };
    PropertySlot m_property_slot { 0 };
    u32 m_extract_index { 0 };
    IteratorHint m_iterator_hint { IteratorHint::Sync };
    FunctionNode const* m_function_node { nullptr };
    ClassExpression const* m_class_expression { nullptr };
    Optional<Bytecode::IdentifierTableIndex> m_lhs_name;
    Optional<Bytecode::IdentifierTableIndex> m_base_identifier;

    // For CreateVariable
    Bytecode::Op::EnvironmentMode m_environment_mode { Bytecode::Op::EnvironmentMode::Lexical };
    bool m_is_immutable { false };
    bool m_is_global { false };
    bool m_is_strict { false };

    // For CreateLexicalEnvironment
    u32 m_capacity { 0 };

    // For NewRegExp
    Bytecode::StringTableIndex m_regex_source_index;
    Bytecode::StringTableIndex m_regex_flags_index;
    Bytecode::RegexTableIndex m_regex_index;

    // For CallDirectEval and other calls
    Optional<Bytecode::StringTableIndex> m_expression_string;

    // For CallBuiltin
    Bytecode::Builtin m_builtin { Bytecode::Builtin::__Count };

    // For CreateArguments
    Bytecode::Op::ArgumentsKind m_arguments_kind { Bytecode::Op::ArgumentsKind::Mapped };
    bool m_create_arguments_needs_dst { true };

    // For CreateRestParams
    u32 m_rest_index { 0 };

    // For SuperCallWithArgumentArray
    bool m_is_synthetic { false };

    // For ArrayAppend
    bool m_is_spread { false };

    // For SetCompletionType / AsyncIteratorClose
    Completion::Type m_completion_type { Completion::Type::Normal };

    // For NewTypeError
    Bytecode::StringTableIndex m_string_table_index;

    // Source location
    Optional<Bytecode::SourceRecord> m_source_record;
};

// TerminatorInstruction is used for control flow instructions that end a basic block.
// Only terminators have CFG targets (true_target/false_target).
// This compile-time separation prevents accidentally setting targets on non-terminators.
class JS_API TerminatorInstruction : public Instruction {
public:
    template<Opcode Op>
    [[nodiscard]] static NonnullOwnPtr<TerminatorInstruction> create()
    {
        static_assert(is_terminator_opcode(Op),
            "TerminatorInstruction::create<Op>() requires a terminator opcode.");
        static_assert(!requires_specialized_instruction(Op),
            "This opcode requires a specialized instruction class. "
            "Use JumpInstruction or BranchInstruction instead.");
        return adopt_own(*new TerminatorInstruction(Op));
    }

    Optional<BlockIndex> true_target_index() const { return m_true_target; }
    Optional<BlockIndex> false_target_index() const { return m_false_target; }
    BasicBlock* true_target() const;
    BasicBlock* false_target() const;

protected:
    explicit TerminatorInstruction(Opcode opcode);

private:
    friend class Builder;
    friend class CFG;
    friend class JumpInstruction;
    friend class BranchInstruction;

    // All target mutation goes through CFG helpers or subclass-specific APIs.
    void set_true_target(Optional<BlockIndex> block) { m_true_target = block; }
    void set_false_target(Optional<BlockIndex> block) { m_false_target = block; }
    void set_true_target(BasicBlock* block);
    void set_false_target(BasicBlock* block);

    Optional<BlockIndex> m_true_target;
    Optional<BlockIndex> m_false_target;
};

// JumpInstruction: Unconditional jump to a single target.
// Also used for ContinuePendingUnwind and EnterUnwindContext (same structure: no operands, one target).
// Operands: none
// Target: exactly one (true_target)
class JS_API JumpInstruction final : public TerminatorInstruction {
public:
    // Target is required at construction for compile-time safety.
    [[nodiscard]] static NonnullOwnPtr<JumpInstruction> create(BasicBlock& target);
    [[nodiscard]] static NonnullOwnPtr<JumpInstruction> create_continue_pending_unwind(BasicBlock& resume_target);
    [[nodiscard]] static NonnullOwnPtr<JumpInstruction> create_enter_unwind_context(BasicBlock& entry_point);

    BasicBlock& target() const
    {
        VERIFY(true_target());
        return *true_target();
    }

private:
    friend class CFG;

    void set_target(BasicBlock& block);

    JumpInstruction(Opcode opcode, BasicBlock& target);
};

// BranchInstruction: Conditional branch with true and false targets.
// Operands: exactly one (the condition)
// Targets: true_target and false_target
class JS_API BranchInstruction final : public TerminatorInstruction {
public:
    // Both targets are required at construction for compile-time safety.
    [[nodiscard]] static NonnullOwnPtr<BranchInstruction> create(Value* condition, BasicBlock& true_target, BasicBlock& false_target);

    Value* condition() const
    {
        VERIFY(operand_count() > 0);
        return operand(0);
    }

    BasicBlock& true_branch() const
    {
        VERIFY(true_target());
        return *true_target();
    }

    BasicBlock& false_branch() const
    {
        VERIFY(false_target());
        return *false_target();
    }

private:
    friend class CFG;

    void set_true_branch(BasicBlock& block);
    void set_false_branch(BasicBlock& block);

    BranchInstruction(Value* condition, BasicBlock& true_target, BasicBlock& false_target);
};

// PhiInstruction: SSA phi node for merging values from different predecessors.
// Stores (predecessor block, value) pairs that must always be kept in sync.
// Result: the merged value
class JS_API PhiInstruction final : public Instruction {
public:
    [[nodiscard]] static NonnullOwnPtr<PhiInstruction> create();

    // Read-only accessors
    size_t incoming_count() const { return phi_predecessors().size(); }
    BlockIndex incoming_block(size_t index) const { return phi_predecessors()[index]; }
    Value* incoming_value(size_t index) const { return operand(index); }

    // Lookup helpers - find the value/index for a given predecessor block
    Value* incoming_value_for(BlockIndex predecessor) const;
    Value* incoming_value_for(BasicBlock const& predecessor) const;
    void set_incoming_value_for(BlockIndex predecessor, Value* value);
    void set_incoming_value_for(BasicBlock const& predecessor, Value* value);

    // Atomic modification methods - these keep predecessor/value pairs in sync
    void add_incoming(BlockIndex predecessor, Value* value);
    void remove_incoming(size_t index);
    void remove_incoming_from(BlockIndex predecessor);
    void set_incoming_block(size_t index, BlockIndex block);
    void set_incoming_value(size_t index, Value* value);

private:
    PhiInstruction();

    // Hide dangerous base class methods that could desync predecessor/value pairs
    using Instruction::add_phi_operand;
    using Instruction::remove_phi_operand;
    using Instruction::set_phi_predecessor;

    // Classes that need internal access to phi operations
    friend class Builder;
    friend class Lifter;
};

// ParallelCopyInstruction: Pseudo-instruction for SSA destruction.
// Represents a set of parallel copies (phi moves) that must all read their
// sources before writing any destinations. The lowerer resolves this into
// an ordered sequence of Mov bytecodes.
// Operands: the source values (tracked for use-def chains)
// Destinations: stored in m_copies (phi results, not tracked as operands)
class JS_API ParallelCopyInstruction final : public Instruction {
public:
    struct Copy {
        ValueIndex dst;
        ValueIndex src;
    };

    [[nodiscard]] static NonnullOwnPtr<ParallelCopyInstruction> create();

    Vector<Copy> const& copies() const { return m_copies; }
    void add_copy(Value* dst, Value* src);
    Value* copy_dst(size_t index) const;
    Value* copy_src(size_t index) const;

private:
    ParallelCopyInstruction();

    Vector<Copy> m_copies;
};

// Check if an opcode is a call-like opcode (has callee and this_value operands)
constexpr bool is_call_opcode(Opcode opcode)
{
    return opcode_traits(opcode).is_call_like;
}

// CallInstruction: Function call with callee, this_value, and arguments.
// Operands: [0] callee, [1] this_value, [2..] arguments
// Result: the return value of the call
class JS_API CallInstruction final : public Instruction {
public:
    [[nodiscard]] static NonnullOwnPtr<CallInstruction> create(Opcode opcode, Value* callee, Value* this_value);

    Value* callee() const
    {
        VERIFY(operand_count() >= 2);
        return operand(0);
    }

    Value* this_value() const
    {
        VERIFY(operand_count() >= 2);
        return operand(1);
    }

    size_t argument_count() const
    {
        return operand_count() > 2 ? operand_count() - 2 : 0;
    }

    Value* argument(size_t index) const
    {
        VERIFY(index + 2 < operand_count());
        return operand(index + 2);
    }

private:
    CallInstruction(Opcode opcode, Value* callee, Value* this_value);
};

// GetByIdInstruction: Property access by identifier.
// Operands: [0] base object
// Required metadata: property_key_index
// Optional metadata: base_identifier
class JS_API GetByIdInstruction final : public Instruction {
public:
    [[nodiscard]] static NonnullOwnPtr<GetByIdInstruction> create(Value* base, Bytecode::PropertyKeyTableIndex property);

    Value* base() const
    {
        VERIFY(operand_count() > 0);
        return operand(0);
    }

    Bytecode::PropertyKeyTableIndex property() const
    {
        return property_key_index();
    }

private:
    GetByIdInstruction(Value* base, Bytecode::PropertyKeyTableIndex property);
};

// BinaryOpInstruction: Binary arithmetic/comparison operations.
// Operands: [0] lhs, [1] rhs
// Fixed arity: exactly 2 operands
class JS_API BinaryOpInstruction final : public Instruction {
public:
    [[nodiscard]] static NonnullOwnPtr<BinaryOpInstruction> create(Opcode opcode, Value* lhs, Value* rhs);

    Value* lhs() const
    {
        VERIFY(operand_count() == 2);
        return operand(0);
    }

    Value* rhs() const
    {
        VERIFY(operand_count() == 2);
        return operand(1);
    }

private:
    BinaryOpInstruction(Opcode opcode, Value* lhs, Value* rhs);
};

// UnaryOpInstruction: Unary operations.
// Operands: [0] operand
// Fixed arity: exactly 1 operand
class JS_API UnaryOpInstruction final : public Instruction {
public:
    [[nodiscard]] static NonnullOwnPtr<UnaryOpInstruction> create(Opcode opcode, Value* operand);

    Value* unary_operand() const
    {
        VERIFY(operand_count() == 1);
        return operand(0);
    }

private:
    UnaryOpInstruction(Opcode opcode, Value* operand);
};

}
