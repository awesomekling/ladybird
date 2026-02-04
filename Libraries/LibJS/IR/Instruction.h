/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/Bytecode/Builtins.h>
#include <LibJS/Bytecode/IdentifierTable.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/PropertyKeyTable.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/Runtime/Iterator.h>

namespace JS {

class ClassExpression;
class FunctionNode;

}

namespace JS::IR {

enum class Opcode : u8 {
    // Control flow
    Jump,
    Branch,
    Return,
    End,
    Throw,

    // SSA
    Phi,

    // Constants
    LoadConstant,
    LoadUndefined,
    LoadNull,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Negate,
    UnaryPlus,

    // Bitwise
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseNot,
    LeftShift,
    RightShift,
    UnsignedRightShift,

    // Comparison
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    LooselyEquals,
    StrictlyEquals,
    LooselyInequals,
    StrictlyInequals,

    // Type ops
    Typeof,
    TypeofBinding,
    ToBoolean,
    ToNumber,
    ToString,
    ToObject,
    ToInt32,
    ToLength,
    Not,
    IsUndefined,
    IsNullish,

    // Increment/Decrement
    Increment,
    Decrement,
    PostfixIncrement,
    PostfixDecrement,

    // String ops
    ConcatString,

    // Property access
    GetById,
    GetByIdWithThis,
    GetByValue,
    GetByValueWithThis,
    GetLength,
    PutById,
    PutByValue,
    DeleteById,
    DeleteByValue,
    HasProperty,
    GetPrivateById,
    PutPrivateById,
    PutGetterById,
    PutSetterById,
    PutPrototypeById,
    PutGetterByIdWithThis,
    PutSetterByIdWithThis,
    PutPrototypeByIdWithThis,
    PutGetterByValue,
    PutSetterByValue,
    PutPrototypeByValue,
    PutGetterByValueWithThis,
    PutSetterByValueWithThis,
    PutPrototypeByValueWithThis,
    PutBySpread,

    // Calls
    Call,
    CallBuiltin,
    CallDirectEval,
    CallWithArgumentArray,
    Construct,
    ConstructWithArgumentArray,
    SuperCallWithArgumentArray,
    ImportCall,

    // Environment
    GetCalleeAndThisFromEnvironment,
    CreateVariable,
    CreateLexicalEnvironment,
    CreateMutableBinding,
    CreateImmutableBinding,
    LeaveLexicalEnvironment,
    EnterObjectEnvironment,
    GetBinding,
    InitializeBinding,
    SetBinding,
    GetGlobal,
    SetGlobal,
    DeleteVariable,
    ResolveThisBinding,
    ResolveSuperBase,

    // Object creation
    NewObject,
    NewArray,
    NewArrayWithLength,
    ArrayAppend,
    NewClass,
    NewFunction,
    NewRegExp,
    InitObjectLiteralProperty,
    CacheObjectShape,

    // Special
    In,
    InstanceOf,

    // Iterators
    GetIterator,
    IteratorNext,
    IteratorNextUnpack,
    IteratorClose,
    IteratorToArray,

    // Generators/Async
    Yield,
    Await,
    GetCompletionFields,

    // Copy
    Move,

    // Tuple extraction (for multi-output instructions)
    ExtractValue,

    // Arguments
    CreateArguments,
    CreateRestParams,

    // New target
    GetNewTarget,

    // Guard operations (may throw but produce no value)
    ThrowIfNotObject,
    ThrowIfNullish,
    ThrowIfTDZ,
};

constexpr bool is_terminator_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Jump:
    case Opcode::Branch:
    case Opcode::Return:
    case Opcode::End:
    case Opcode::Throw:
    case Opcode::Yield:
    case Opcode::Await:
        return true;
    default:
        return false;
    }
}

constexpr bool may_throw_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Jump:
    case Opcode::Branch:
    case Opcode::Return:
    case Opcode::End:
    case Opcode::Phi:
    case Opcode::LoadConstant:
    case Opcode::LoadUndefined:
    case Opcode::LoadNull:
    case Opcode::ToBoolean:
    case Opcode::Not:
    case Opcode::Typeof:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
    case Opcode::Move:
    case Opcode::ExtractValue:
    case Opcode::InitObjectLiteralProperty:
    case Opcode::CacheObjectShape:
    case Opcode::CreateLexicalEnvironment:
    case Opcode::CreateMutableBinding:
    case Opcode::CreateImmutableBinding:
    case Opcode::LeaveLexicalEnvironment:
    case Opcode::CreateArguments:
    case Opcode::CreateRestParams:
        return false;
    default:
        return true;
    }
}

// Does this opcode have observable side effects at the opcode level?
// NB: This is conservative - arithmetic/comparison ops CAN have side effects
// when operands are objects (ToPrimitive calls valueOf/toString).
// Use Instruction::has_side_effects() for type-aware analysis.
constexpr bool has_side_effects_opcode(Opcode opcode)
{
    switch (opcode) {
    // These are always side-effect free regardless of operand types
    case Opcode::Not:
    case Opcode::Move:
    case Opcode::Phi:
    case Opcode::LoadConstant:
    case Opcode::LoadUndefined:
    case Opcode::LoadNull:
    case Opcode::ToBoolean:
    case Opcode::Typeof:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
    case Opcode::ExtractValue:
    // Strict equality never calls user code
    case Opcode::StrictlyEquals:
    case Opcode::StrictlyInequals:
        return false;
    // Everything else potentially has side effects (including arithmetic
    // which can call ToPrimitive on object operands)
    default:
        return true;
    }
}

// Is this opcode pure (no side effects, deterministic) at the opcode level?
// NB: This is conservative - arithmetic/comparison ops are NOT pure when
// operands could be objects. Use Instruction::is_pure() for type-aware analysis.
constexpr bool is_pure_opcode(Opcode opcode)
{
    switch (opcode) {
    // These are always pure regardless of operand types
    case Opcode::Not:
    case Opcode::ToBoolean:
    case Opcode::Typeof:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
    // Strict equality is pure (no type coercion)
    case Opcode::StrictlyEquals:
    case Opcode::StrictlyInequals:
        return true;
    default:
        return false;
    }
}

// Can this instruction be safely hoisted out of a loop at the opcode level?
// Must be pure AND not throw. NB: This is conservative - arithmetic ops can
// call ToPrimitive on objects. Use Instruction::is_hoistable() for type-aware analysis.
constexpr bool is_hoistable_opcode(Opcode opcode)
{
    switch (opcode) {
    // These are always safe to hoist regardless of operand types
    case Opcode::Not:
    case Opcode::ToBoolean:
    case Opcode::Typeof:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
        return true;
    // Everything else needs type-aware analysis
    default:
        return false;
    }
}

JS_API char const* opcode_to_string(Opcode opcode);

class TerminatorInstruction;

class JS_API Instruction {
    AK_MAKE_NONCOPYABLE(Instruction);
    AK_MAKE_NONMOVABLE(Instruction);

public:
    static NonnullOwnPtr<Instruction> create(Opcode opcode);

    virtual ~Instruction() = default;

    Opcode opcode() const { return m_opcode; }

    // Safe opcode mutation for comparison inversion.
    // Returns true if the opcode was changed, false if not a comparison.
    // This is the only safe way to mutate an opcode in-place.
    bool try_invert_comparison();

    BasicBlock* parent_block() const { return m_parent_block; }
    void set_parent_block(BasicBlock* block) { m_parent_block = block; }

    Value* result() const { return m_result; }
    void set_result(Value* value);

    Vector<Value*> const& operands() const { return m_operands; }
    void add_operand(Value* value);
    void set_operand(size_t index, Value* value);
    void clear_operand_uses();

    // For Phi nodes
    Vector<BasicBlock*> const& phi_predecessors() const { return m_phi_predecessors; }
    void add_phi_operand(BasicBlock* predecessor, Value* value);
    void set_phi_predecessor(size_t index, BasicBlock* block) { m_phi_predecessors[index] = block; }
    void remove_phi_operand(size_t index)
    {
        m_phi_predecessors.remove(index);
        m_operands.remove(index);
    }

    // Instruction-specific indices (reuse bytecode tables)
    Bytecode::PropertyKeyTableIndex property_key_index() const { return m_property_key_index; }
    void set_property_key_index(Bytecode::PropertyKeyTableIndex index) { m_property_key_index = index; }

    Bytecode::IdentifierTableIndex identifier_index() const { return m_identifier_index; }
    void set_identifier_index(Bytecode::IdentifierTableIndex index) { m_identifier_index = index; }

    u32 cache_index() const { return m_cache_index; }
    void set_cache_index(u32 index) { m_cache_index = index; }

    u32 property_slot() const { return m_property_slot; }
    void set_property_slot(u32 slot) { m_property_slot = slot; }

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

    // For CreateRestParams
    u32 rest_index() const { return m_rest_index; }
    void set_rest_index(u32 index) { m_rest_index = index; }

    // For SuperCallWithArgumentArray
    bool is_synthetic() const { return m_is_synthetic; }
    void set_is_synthetic(bool value) { m_is_synthetic = value; }

    // For ArrayAppend
    bool is_spread() const { return m_is_spread; }
    void set_is_spread(bool value) { m_is_spread = value; }

    bool is_terminator() const { return is_terminator_opcode(m_opcode); }
    bool may_throw() const { return may_throw_opcode(m_opcode); }

    // Type-aware effect analysis (examines operand types for safe primitives)
    bool has_side_effects() const;
    bool is_pure() const;
    bool is_hoistable() const;

protected:
    explicit Instruction(Opcode opcode);

private:
    Opcode m_opcode;
    BasicBlock* m_parent_block { nullptr };
    Value* m_result { nullptr };
    Vector<Value*> m_operands;

    // For Phi
    Vector<BasicBlock*> m_phi_predecessors;

    // Instruction-specific indices
    Bytecode::PropertyKeyTableIndex m_property_key_index;
    Bytecode::IdentifierTableIndex m_identifier_index;
    u32 m_cache_index { 0 };
    u32 m_property_slot { 0 };
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

    // For CreateRestParams
    u32 m_rest_index { 0 };

    // For SuperCallWithArgumentArray
    bool m_is_synthetic { false };

    // For ArrayAppend
    bool m_is_spread { false };
};

// TerminatorInstruction is used for control flow instructions that end a basic block.
// Only terminators have CFG targets (true_target/false_target).
// This compile-time separation prevents accidentally setting targets on non-terminators.
class JS_API TerminatorInstruction : public Instruction {
public:
    static NonnullOwnPtr<TerminatorInstruction> create(Opcode opcode);

    BasicBlock* true_target() const { return m_true_target; }
    BasicBlock* false_target() const { return m_false_target; }
    void set_true_target(BasicBlock* block) { m_true_target = block; }
    void set_false_target(BasicBlock* block) { m_false_target = block; }

protected:
    explicit TerminatorInstruction(Opcode opcode);

private:
    BasicBlock* m_true_target { nullptr };
    BasicBlock* m_false_target { nullptr };
};

// JumpInstruction: Unconditional jump to a single target.
// Operands: none
// Target: exactly one (true_target)
class JS_API JumpInstruction final : public TerminatorInstruction {
public:
    // Target is required at construction for compile-time safety.
    static NonnullOwnPtr<JumpInstruction> create(BasicBlock& target);

    BasicBlock& target() const
    {
        VERIFY(true_target());
        return *true_target();
    }
    void set_target(BasicBlock& block) { set_true_target(&block); }

private:
    explicit JumpInstruction(BasicBlock& target);
};

// BranchInstruction: Conditional branch with true and false targets.
// Operands: exactly one (the condition)
// Targets: true_target and false_target
class JS_API BranchInstruction final : public TerminatorInstruction {
public:
    // Both targets are required at construction for compile-time safety.
    static NonnullOwnPtr<BranchInstruction> create(Value* condition, BasicBlock& true_target, BasicBlock& false_target);

    Value* condition() const
    {
        VERIFY(!operands().is_empty());
        return operands()[0];
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

    void set_true_branch(BasicBlock& block) { set_true_target(&block); }
    void set_false_branch(BasicBlock& block) { set_false_target(&block); }

private:
    BranchInstruction(Value* condition, BasicBlock& true_target, BasicBlock& false_target);
};

// PhiInstruction: SSA phi node for merging values from different predecessors.
// Operands: one value per predecessor (in phi_predecessors order)
// Result: the merged value
class JS_API PhiInstruction final : public Instruction {
public:
    static NonnullOwnPtr<PhiInstruction> create();

    // Typed accessors for phi-specific functionality
    size_t incoming_count() const { return phi_predecessors().size(); }
    BasicBlock* incoming_block(size_t index) const { return phi_predecessors()[index]; }
    Value* incoming_value(size_t index) const { return operands()[index]; }

private:
    PhiInstruction();
};

// Check if an opcode is a call-like opcode (has callee and this_value operands)
constexpr bool is_call_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Call:
    case Opcode::CallBuiltin:
    case Opcode::CallDirectEval:
    case Opcode::CallWithArgumentArray:
        return true;
    default:
        return false;
    }
}

// CallInstruction: Function call with callee, this_value, and arguments.
// Operands: [0] callee, [1] this_value, [2..] arguments
// Result: the return value of the call
class JS_API CallInstruction final : public Instruction {
public:
    static NonnullOwnPtr<CallInstruction> create(Opcode opcode, Value* callee, Value* this_value);

    Value* callee() const
    {
        VERIFY(operands().size() >= 2);
        return operands()[0];
    }

    Value* this_value() const
    {
        VERIFY(operands().size() >= 2);
        return operands()[1];
    }

    size_t argument_count() const
    {
        return operands().size() > 2 ? operands().size() - 2 : 0;
    }

    Value* argument(size_t index) const
    {
        VERIFY(index + 2 < operands().size());
        return operands()[index + 2];
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
    static NonnullOwnPtr<GetByIdInstruction> create(Value* base, Bytecode::PropertyKeyTableIndex property);

    Value* base() const
    {
        VERIFY(!operands().is_empty());
        return operands()[0];
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
    static NonnullOwnPtr<BinaryOpInstruction> create(Opcode opcode, Value* lhs, Value* rhs);

    Value* lhs() const
    {
        VERIFY(operands().size() == 2);
        return operands()[0];
    }

    Value* rhs() const
    {
        VERIFY(operands().size() == 2);
        return operands()[1];
    }

private:
    BinaryOpInstruction(Opcode opcode, Value* lhs, Value* rhs);
};

// UnaryOpInstruction: Unary operations.
// Operands: [0] operand
// Fixed arity: exactly 1 operand
class JS_API UnaryOpInstruction final : public Instruction {
public:
    static NonnullOwnPtr<UnaryOpInstruction> create(Opcode opcode, Value* operand);

    Value* operand() const
    {
        VERIFY(operands().size() == 1);
        return operands()[0];
    }

private:
    UnaryOpInstruction(Opcode opcode, Value* operand);
};

}
