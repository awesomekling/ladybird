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
//          tuple_arity
//
// The string name is auto-derived by stringifying the identifier.

// clang-format off
#define IR_OPCODE_LIST(OP) \
    /* Control flow */                                                                                                             \
    OP(Jump,                            true,  E_NONE,       false, false, 0,   Type::Unknown,   false, 0) \
    OP(Branch,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(Return,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(End,                             true,  E_NONE,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(Throw,                           true,  E_THROW,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(ContinuePendingUnwind,           true,  E_THROW,      false, false, 0,   Type::Unknown,   false, 0) \
    /* SSA */                                                                                                                      \
    OP(Phi,                             false, E_NONE,       false, true,  255, Type::Unknown,   false, 0) \
    OP(ParallelCopy,                    false, E_WRITE,      false, false, 255, Type::Unknown,   false, 0) \
    /* Constants */                                                                                                                \
    OP(LoadConstant,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(LoadUndefined,                   false, E_NONE,       false, true,  0,   Type::Undefined, false, 0) \
    OP(LoadNull,                        false, E_NONE,       false, true,  0,   Type::Null,      false, 0) \
    /* Arithmetic (may call ToPrimitive on objects) */                                                                             \
    OP(Add,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true,  0) \
    OP(Sub,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(Mul,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true,  0) \
    OP(Div,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(Mod,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(Exp,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(Negate,                          false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    OP(UnaryPlus,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    /* Bitwise (may call ToInt32 -> ToPrimitive on objects) */                                                                     \
    OP(BitwiseAnd,                      false, E_CALL,       false, true,  2,   Type::Int32,     true,  0) \
    OP(BitwiseOr,                       false, E_CALL,       false, true,  2,   Type::Int32,     true,  0) \
    OP(BitwiseXor,                      false, E_CALL,       false, true,  2,   Type::Int32,     true,  0) \
    OP(BitwiseNot,                      false, E_CALL,       false, true,  1,   Type::Int32,     false, 0) \
    OP(LeftShift,                       false, E_CALL,       false, true,  2,   Type::Int32,     false, 0) \
    OP(RightShift,                      false, E_CALL,       false, true,  2,   Type::Int32,     false, 0) \
    OP(UnsignedRightShift,              false, E_CALL,       false, true,  2,   Type::Number,    false, 0) \
    /* Comparison (relational may call ToPrimitive, loose equality too) */                                                         \
    OP(LessThan,                        false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(LessThanEquals,                  false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(GreaterThan,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(GreaterThanEquals,               false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(LooselyEquals,                   false, E_CALL,       false, true,  2,   Type::Boolean,   true,  0) \
    OP(StrictlyEquals,                  false, E_NONE,       false, true,  2,   Type::Boolean,   true,  0) \
    OP(LooselyInequals,                 false, E_CALL,       false, true,  2,   Type::Boolean,   true,  0) \
    OP(StrictlyInequals,                false, E_NONE,       false, true,  2,   Type::Boolean,   true,  0) \
    /* Type ops */                                                                                                                 \
    OP(Typeof,                          false, E_NONE,       false, true,  1,   Type::String,    false, 0) \
    OP(TypeofBinding,                   false, E_CALL,       false, true,  0,   Type::String,    false, 0) \
    OP(ToBoolean,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0) \
    OP(ToNumber,                        false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    OP(ToNumeric,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(ToString,                        false, E_CALL,       false, true,  1,   Type::String,    false, 0) \
    OP(ToObject,                        false, E_THROW,      false, true,  1,   Type::Object,    false, 0) \
    OP(ToInt32,                         false, E_CALL,       false, true,  1,   Type::Int32,     false, 0) \
    OP(ToLength,                        false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(Not,                             false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0) \
    OP(IsUndefined,                     false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0) \
    OP(IsNullish,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false, 0) \
    /* Increment/Decrement (may call ToNumber -> ToPrimitive) */                                                                   \
    OP(Increment,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    OP(Decrement,                       false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    OP(PostfixIncrement,                false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    OP(PostfixDecrement,                false, E_CALL,       false, true,  1,   Type::Number,    false, 0) \
    /* String ops */                                                                                                               \
    OP(ConcatString,                    false, E_CALL,       false, true,  2,   Type::String,    false, 0) \
    /* Property access */                                                                                                          \
    OP(GetById,                         false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(GetByIdWithThis,                 false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(GetByValue,                      false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    OP(GetByValueWithThis,              false, E_CALL,       false, true,  3,   Type::Unknown,   false, 0) \
    OP(GetLength,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(PutById,                         false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    OP(PutByIdWithThis,                 false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutByValue,                      false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutByValueWithThis,              false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0) \
    OP(DeleteById,                      false, E_CALL,       false, true,  1,   Type::Boolean,   false, 0) \
    OP(DeleteByIdWithThis,              false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(DeleteByValue,                   false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(DeleteByValueWithThis,           false, E_CALL,       false, true,  3,   Type::Boolean,   false, 0) \
    OP(HasProperty,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(HasPrivateId,                    false, E_CALL,       false, true,  1,   Type::Boolean,   false, 0) \
    OP(GetPrivateById,                  false, E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(PutPrivateById,                  false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    OP(PutGetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    OP(PutSetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    OP(PutPrototypeById,                false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    OP(PutGetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutSetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutPrototypeByIdWithThis,        false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutGetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutSetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutPrototypeByValue,             false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(PutGetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0) \
    OP(PutSetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0) \
    OP(PutPrototypeByValueWithThis,     false, E_CALL,       false, false, 4,   Type::Unknown,   false, 0) \
    OP(PutBySpread,                     false, E_CALL,       false, false, 2,   Type::Unknown,   false, 0) \
    /* Calls */                                                                                                                    \
    OP(Call,                            false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0) \
    OP(CallBuiltin,                     false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0) \
    OP(CallDirectEval,                  false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0) \
    OP(CallWithArgumentArray,           false, E_CALL,       true,  true,  255, Type::Unknown,   false, 0) \
    OP(Construct,                       false, E_CALL,       false, true,  255, Type::Unknown,   false, 0) \
    OP(ConstructWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Unknown,   false, 0) \
    OP(SuperCallWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Unknown,   false, 0) \
    OP(ImportCall,                      false, E_CALL,       false, true,  2,   Type::Unknown,   false, 0) \
    /* Environment */                                                                                                              \
    OP(GetCalleeAndThisFromEnvironment, false, E_CALL,       false, true,  0,   Type::Unknown,   false, 2) \
    OP(CreateVariable,                  false, E_CALL,       false, false, 0,   Type::Unknown,   false, 0) \
    OP(CreateLexicalEnvironment,        false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    OP(CreateMutableBinding,            false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(CreateImmutableBinding,          false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(LeaveLexicalEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(EnterObjectEnvironment,          false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(GetBinding,                      false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0) \
    OP(InitializeBinding,               false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(SetBinding,                      false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(GetGlobal,                       false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0) \
    OP(SetGlobal,                       false, E_CALL,       false, false, 1,   Type::Unknown,   false, 0) \
    OP(DeleteVariable,                  false, E_CALL,       false, true,  0,   Type::Boolean,   false, 0) \
    OP(ResolveThisBinding,              false, E_CALL,       false, false, 0,   Type::Unknown,   false, 0) \
    OP(ResolveSuperBase,                false, E_CALL,       false, true,  0,   Type::Unknown,   false, 0) \
    OP(CreatePrivateEnvironment,        false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(LeavePrivateEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(AddPrivateName,                  false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(CreateVariableEnvironment,       false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    /* Object creation */                                                                                                          \
    OP(NewObject,                       false, E_THROW,      false, true,  0,   Type::Object,    false, 0) \
    OP(NewArray,                        false, E_THROW,      false, true,  255, Type::Array,     false, 0) \
    OP(NewArrayWithLength,              false, E_THROW,      false, true,  1,   Type::Array,     false, 0) \
    OP(ArrayAppend,                     false, E_THROW_WRITE,false, false, 2,   Type::Unknown,   false, 0) \
    OP(NewClass,                        false, E_CALL,       false, true,  255, Type::Function,  false, 0) \
    OP(NewFunction,                     false, E_THROW,      false, true,  255, Type::Function,  false, 0) \
    OP(NewRegExp,                       false, E_THROW,      false, true,  0,   Type::Object,    false, 0) \
    OP(GetTemplateObject,               false, E_WRITE,      false, true,  255, Type::Array,     false, 0) \
    OP(InitObjectLiteralProperty,       false, E_WRITE,      false, false, 2,   Type::Unknown,   false, 0) \
    OP(CacheObjectShape,                false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(CopyObjectExcludingProperties,   false, E_CALL,       false, true,  255, Type::Unknown,   false, 0) \
    /* Special */                                                                                                                  \
    OP(In,                              false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    OP(InstanceOf,                      false, E_CALL,       false, true,  2,   Type::Boolean,   false, 0) \
    /* Iterators */                                                                                                                \
    OP(GetIterator,                     false, E_CALL,       false, true,  1,   Type::Unknown,   false, 3) \
    OP(GetObjectPropertyIterator,       false, E_CALL,       false, true,  1,   Type::Unknown,   false, 3) \
    OP(IteratorNext,                    false, E_CALL,       false, true,  3,   Type::Unknown,   false, 0) \
    OP(IteratorNextUnpack,              false, E_CALL,       false, true,  3,   Type::Unknown,   false, 2) \
    OP(IteratorClose,                   false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(AsyncIteratorClose,              false, E_CALL,       false, false, 3,   Type::Unknown,   false, 0) \
    OP(IteratorToArray,                 false, E_CALL,       false, true,  3,   Type::Array,     false, 0) \
    /* Generators/Async */                                                                                                         \
    OP(Yield,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(Await,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false, 0) \
    OP(PrepareYield,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(GetCompletionFields,             false, E_THROW,      false, true,  1,   Type::Unknown,   false, 2) \
    OP(SetCompletionType,               false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(NewTypeError,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    /* Copy */                                                                                                                     \
    OP(Move,                            false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0) \
    /* Tuple extraction (for multi-output instructions) */                                                                         \
    OP(ExtractValue,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false, 0) \
    /* Arguments */                                                                                                                \
    OP(CreateArguments,                 false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    OP(CreateRestParams,                false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    /* New target */                                                                                                               \
    OP(GetNewTarget,                    false, E_THROW,      false, true,  0,   Type::Unknown,   false, 0) \
    /* Exception handling */                                                                                                       \
    OP(Catch,                           false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    OP(EnterUnwindContext,              true,  E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(LeaveUnwindContext,              false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(ScheduleJump,                    true,  E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(LeaveFinally,                    false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(RestoreScheduledJump,            false, E_WRITE,      false, false, 0,   Type::Unknown,   false, 0) \
    OP(SetSavedReturnValue,             false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(GetException,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false, 0) \
    OP(SetException,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false, 0) \
    /* Guard operations (may throw but produce no value) */                                                                        \
    OP(ThrowIfNotObject,                false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(ThrowIfNullish,                  false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0) \
    OP(ThrowIfTDZ,                      false, E_THROW,      false, false, 1,   Type::Unknown,   false, 0)
// clang-format on
