/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

// Single source of truth for all IR opcodes and their static metadata.
// Every opcode is defined exactly once here. The enum, traits table,
// and all derived queries are generated from this list.
//
// Columns: name, is_terminator, effects, is_call_like, has_result,
//          operand_arity, guaranteed_result_type, is_commutative,
//          tuple_arity, effect_refinement
//
// The string name is auto-derived by stringifying the identifier.

// clang-format off
#define IR_OPCODE_LIST(OP) \
    /* Control flow */                                                                                                                       \
    OP(Jump,                            true,  E_NONE,       false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(Branch,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(Return,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(End,                             true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(Throw,                           true,  E_THROW,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(ContinuePendingUnwind,           true,  E_THROW,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    /* SSA */                                                                                                                                \
    OP(Phi,                             false, E_NONE,       false, true,  255, Type::Unknown,   false, 0, R_NONE) \
    OP(ParallelCopy,                    false, E_WRITE,      false, false, 255, Type::Unknown,   false, 0, R_NONE) \
    /* Constants */                                                                                                                          \
    OP(LoadConstant,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(LoadUndefined,                   false, E_NONE,       false, true,  0,   Type::Undefined, false, 0, R_NONE) \
    OP(LoadNull,                        false, E_NONE,       false, true,  0,   Type::Null,      false, 0, R_NONE) \
    /* Arithmetic (may call ToPrimitive on objects) */                                                                                       \
    OP(Add,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true,  0, R_PRIM) \
    OP(Sub,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_PRIM) \
    OP(Mul,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true,  0, R_PRIM) \
    OP(Div,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_PRIM) \
    OP(Mod,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_PRIM) \
    OP(Exp,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_PRIM) \
    OP(Negate,                          false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    OP(UnaryPlus,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    /* Bitwise (may call ToInt32 -> ToPrimitive on objects) */                                                                               \
    OP(BitwiseAnd,                      false, E_CALL,       false, true,  2,   Type::Int32,     true,  0, R_PRIM) \
    OP(BitwiseOr,                       false, E_CALL,       false, true,  2,   Type::Int32,     true,  0, R_PRIM) \
    OP(BitwiseXor,                      false, E_CALL,       false, true,  2,   Type::Int32,     true,  0, R_PRIM) \
    OP(BitwiseNot,                      false, E_CALL,       false, true,  1,   Type::Int32,     false, 0, R_PRIM) \
    OP(LeftShift,                       false, E_CALL,       false, true,  2,   Type::Int32,     false, 0, R_PRIM) \
    OP(RightShift,                      false, E_CALL,       false, true,  2,   Type::Int32,     false, 0, R_PRIM) \
    OP(UnsignedRightShift,              false, E_CALL,       false, true,  2,   Type::Number,    false, 0, R_PRIM) \
    /* Comparison (relational may call ToPrimitive, loose equality too) */                                                                   \
    OP(LessThan,                        false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_PRIM) \
    OP(LessThanEquals,                  false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_PRIM) \
    OP(GreaterThan,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_PRIM) \
    OP(GreaterThanEquals,               false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_PRIM) \
    OP(LooselyEquals,                   false, E_CALL,       false, true,  2,   Type::Boolean,   true,  0, R_PRIM) \
    OP(StrictlyEquals,                  false, E_NONE,       false, true,  2,   Type::Boolean,   true,  0, R_NONE) \
    OP(LooselyInequals,                 false, E_CALL,       false, true,  2,   Type::Boolean,   true,  0, R_PRIM) \
    OP(StrictlyInequals,                false, E_NONE,       false, true,  2,   Type::Boolean,   true,  0, R_NONE) \
    /* Type ops */                                                                                                                           \
    OP(Typeof,                          false, E_NONE,       false, true,  1,   Type::String,    false, 0, R_NONE) \
    OP(TypeofBinding,                   false, E_CALL,       false, true,  0,   Type::String,    false, 0, R_NONE) \
    OP(ToBoolean,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    OP(ToNumber,                        false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_NONE) \
    OP(ToNumeric,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_PRIM) \
    OP(ToString,                        false, E_CALL,       false, true,  1,   Type::String,    false, 0, R_NONE) \
    OP(ToObject,                        false, E_THROW,      false, true,  1,   Type::Object,    false, 0, R_NONE) \
    OP(ToInt32,                         false, E_CALL,       false, true,  1,   Type::Int32,     false, 0, R_NONE) \
    OP(ToLength,                        false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(Not,                             false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    OP(IsUndefined,                     false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    OP(IsNullish,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    /* Increment/Decrement (may call ToNumber -> ToPrimitive) */                                                                             \
    OP(Increment,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    OP(Decrement,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    OP(PostfixIncrement,                false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    OP(PostfixDecrement,                false, E_CALL,       false, true,  1,   Type::Number,    false, 0, R_PRIM) \
    /* String ops */                                                                                                                         \
    OP(ConcatString,                    false, E_CALL,       false, true,  2,   Type::String,    false, 0, R_NONE) \
    /* Property access */                                                                                                                    \
    OP(GetById,                         false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetByIdWithThis,                 false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetByValue,                      false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetByValueWithThis,              false, E_CALL,       false, true,  3,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetLength,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutById,                         false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutByIdWithThis,                 false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutByValue,                      false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutByValueWithThis,              false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0, R_NONE) \
    OP(DeleteById,                      false, E_CALL,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    OP(DeleteByIdWithThis,              false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_NONE) \
    OP(DeleteByValue,                   false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_NONE) \
    OP(DeleteByValueWithThis,           false, E_CALL,       false, true,  3,   Type::Boolean,   false, 0, R_NONE) \
    OP(HasProperty,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_NONE) \
    OP(HasPrivateId,                    false, E_CALL,       false, true,  1,   Type::Boolean,   false, 0, R_NONE) \
    OP(GetPrivateById,                  false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutPrivateById,                  false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutGetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutSetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutPrototypeById,                false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutGetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutSetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutPrototypeByIdWithThis,        false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutGetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutSetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutPrototypeByValue,             false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutGetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutSetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutPrototypeByValueWithThis,     false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0, R_NONE) \
    OP(PutBySpread,                     false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    /* Calls */                                                                                                                              \
    OP(Call,                            false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0, R_NONE) \
    OP(CallBuiltin,                     false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0, R_NONE) \
    OP(CallDirectEval,                  false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0, R_NONE) \
    OP(CallWithArgumentArray,           false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0, R_NONE) \
    OP(Construct,                       false, E_CALL,       false, true,  255, Type::Object,    false, 0, R_NONE) \
    OP(ConstructWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Object,    false, 0, R_NONE) \
    OP(SuperCallWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Object,    false, 0, R_NONE) \
    OP(ImportCall,                      false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0, R_NONE) \
    /* Environment */                                                                                                                        \
    OP(GetCalleeAndThisFromEnvironment, false, E_CALL,       false, true,  0,   Type::Unknown,   false, 2, R_NONE) \
    OP(CreateVariable,                  false, E_CALL,       false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreateLexicalEnvironment,        false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreateMutableBinding,            false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreateImmutableBinding,          false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(LeaveLexicalEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(EnterObjectEnvironment,          false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetBinding,                      false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(InitializeBinding,               false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(SetBinding,                      false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetGlobal,                       false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(SetGlobal,                       false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(DeleteVariable,                  false, E_CALL,       false, true,  0,   Type::Boolean,   false, 0, R_NONE) \
    OP(ResolveThisBinding,              false, E_CALL,       false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(ResolveSuperBase,                false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreatePrivateEnvironment,        false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(LeavePrivateEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(AddPrivateName,                  false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreateVariableEnvironment,       false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    /* Object creation */                                                                                                                    \
    OP(NewObject,                       false, E_THROW,      false, true,  0,   Type::Object,    false, 0, R_NONE) \
    OP(NewArray,                        false, E_THROW,      false, true,  255, Type::Array,     false, 0, R_NONE) \
    OP(NewArrayWithLength,              false, E_THROW,      false, true,  1,   Type::Array,     false, 0, R_NONE) \
    OP(ArrayAppend,                     false, E_THROW_WRITE,false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(NewClass,                        false, E_CALL,       false, true,  255, Type::Function,  false, 0, R_NONE) \
    OP(NewFunction,                     false, E_THROW,      false, true,  255, Type::Function,  false, 0, R_NONE) \
    OP(NewRegExp,                       false, E_THROW,      false, true,  0,   Type::Object,    false, 0, R_NONE) \
    OP(GetTemplateObject,               false, E_WRITE,      false, true,  255, Type::Array,     false, 0, R_NONE) \
    OP(InitObjectLiteralProperty,       false, E_WRITE,      false, false, 2,   Type::Unknown,   false, 0, R_NONE) \
    OP(CacheObjectShape,                false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(CopyObjectExcludingProperties,   false, E_CALL,       false, true,  255, Type::Object,    false, 0, R_NONE) \
    /* Special */                                                                                                                            \
    OP(In,                              false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_NONE) \
    OP(InstanceOf,                      false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0, R_NONE) \
    /* Iterators */                                                                                                                          \
    OP(GetIterator,                     false, E_CALL,       false, true,  1,   Type::Unknown,   false, 3, R_NONE) \
    OP(GetObjectPropertyIterator,       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 3, R_NONE) \
    OP(IteratorNext,                    false, E_CALL,       false, true,  3,   Type::Unknown,   false, 0, R_NONE) \
    OP(IteratorNextUnpack,              false, E_CALL,       false, true,  3,   Type::Unknown,   false, 2, R_NONE) \
    OP(IteratorClose,                   false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(AsyncIteratorClose,              false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0, R_NONE) \
    OP(IteratorToArray,                 false, E_CALL,       false, true,  3,   Type::Array,     false, 0, R_NONE) \
    /* Generators/Async */                                                                                                                   \
    OP(Yield,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(Await,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    OP(PrepareYield,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetCompletionFields,             false, E_THROW,      false, true,  1,   Type::Unknown,   false, 2, R_NONE) \
    OP(SetCompletionType,               false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(NewTypeError,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    /* Copy */                                                                                                                               \
    OP(Move,                            false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    /* Tuple extraction (for multi-output instructions) */                                                                                   \
    OP(ExtractValue,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0, R_NONE) \
    /* Arguments */                                                                                                                          \
    OP(CreateArguments,                 false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(CreateRestParams,                false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    /* New target */                                                                                                                         \
    OP(GetNewTarget,                    false, E_THROW,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    /* Exception handling */                                                                                                                 \
    OP(Catch,                           false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(EnterUnwindContext,              true,  E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(LeaveUnwindContext,              false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(ScheduleJump,                    true,  E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(LeaveFinally,                    false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(RestoreScheduledJump,            false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0, R_NONE) \
    OP(SetSavedReturnValue,             false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(GetException,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0, R_NONE) \
    OP(SetException,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    /* Guard operations (may throw but produce no value) */                                                                                  \
    OP(ThrowIfNotObject,                false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(ThrowIfNullish,                  false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0, R_NONE) \
    OP(ThrowIfTDZ,                      false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0, R_NONE)
// clang-format on
