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
//          operand_arity, guaranteed_result_type, is_commutative
//
// The string name is auto-derived by stringifying the identifier.

// clang-format off
#define IR_OPCODE_LIST(OP) \
    /* Control flow */                                                                                                          \
    OP(Jump,                            true,  E_NONE,       false, false, 0,   Type::Unknown,   false) \
    OP(Branch,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false) \
    OP(Return,                          true,  E_NONE,       false, false, 1,   Type::Unknown,   false) \
    OP(End,                             true,  E_NONE,       false, false, 1,   Type::Unknown,   false) \
    OP(Throw,                           true,  E_THROW,      false, false, 1,   Type::Unknown,   false) \
    OP(ContinuePendingUnwind,           true,  E_THROW,      false, false, 0,   Type::Unknown,   false) \
    /* SSA */                                                                                                                   \
    OP(Phi,                             false, E_NONE,       false, true,  255, Type::Unknown,   false) \
    OP(ParallelCopy,                    false, E_WRITE,      false, false, 255, Type::Unknown,   false) \
    /* Constants */                                                                                                             \
    OP(LoadConstant,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false) \
    OP(LoadUndefined,                   false, E_NONE,       false, true,  0,   Type::Undefined, false) \
    OP(LoadNull,                        false, E_NONE,       false, true,  0,   Type::Null,      false) \
    /* Arithmetic (may call ToPrimitive on objects) */                                                                          \
    OP(Add,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true ) \
    OP(Sub,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(Mul,                             false, E_CALL,       false, true,  2,   Type::Unknown,   true ) \
    OP(Div,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(Mod,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(Exp,                             false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(Negate,                          false, E_CALL,       false, true,  1,   Type::Number,    false) \
    OP(UnaryPlus,                       false, E_CALL,       false, true,  1,   Type::Number,    false) \
    /* Bitwise (may call ToInt32 -> ToPrimitive on objects) */                                                                  \
    OP(BitwiseAnd,                      false, E_CALL,       false, true,  2,   Type::Int32,     true ) \
    OP(BitwiseOr,                       false, E_CALL,       false, true,  2,   Type::Int32,     true ) \
    OP(BitwiseXor,                      false, E_CALL,       false, true,  2,   Type::Int32,     true ) \
    OP(BitwiseNot,                      false, E_CALL,       false, true,  1,   Type::Int32,     false) \
    OP(LeftShift,                       false, E_CALL,       false, true,  2,   Type::Int32,     false) \
    OP(RightShift,                      false, E_CALL,       false, true,  2,   Type::Int32,     false) \
    OP(UnsignedRightShift,              false, E_CALL,       false, true,  2,   Type::Number,    false) \
    /* Comparison (relational may call ToPrimitive, loose equality too) */                                                      \
    OP(LessThan,                        false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(LessThanEquals,                  false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(GreaterThan,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(GreaterThanEquals,               false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(LooselyEquals,                   false, E_CALL,       false, true,  2,   Type::Boolean,   true ) \
    OP(StrictlyEquals,                  false, E_NONE,       false, true,  2,   Type::Boolean,   true ) \
    OP(LooselyInequals,                 false, E_CALL,       false, true,  2,   Type::Boolean,   true ) \
    OP(StrictlyInequals,                false, E_NONE,       false, true,  2,   Type::Boolean,   true ) \
    /* Type ops */                                                                                                              \
    OP(Typeof,                          false, E_NONE,       false, true,  1,   Type::String,    false) \
    OP(TypeofBinding,                   false, E_CALL,       false, true,  0,   Type::String,    false) \
    OP(ToBoolean,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false) \
    OP(ToNumber,                        false, E_CALL,       false, true,  1,   Type::Number,    false) \
    OP(ToNumeric,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(ToString,                        false, E_CALL,       false, true,  1,   Type::String,    false) \
    OP(ToObject,                        false, E_THROW,      false, true,  1,   Type::Object,    false) \
    OP(ToInt32,                         false, E_CALL,       false, true,  1,   Type::Int32,     false) \
    OP(ToLength,                        false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(Not,                             false, E_NONE,       false, true,  1,   Type::Boolean,   false) \
    OP(IsUndefined,                     false, E_NONE,       false, true,  1,   Type::Boolean,   false) \
    OP(IsNullish,                       false, E_NONE,       false, true,  1,   Type::Boolean,   false) \
    /* Increment/Decrement (may call ToNumber -> ToPrimitive) */                                                                \
    OP(Increment,                       false, E_CALL,       false, true,  1,   Type::Number,    false) \
    OP(Decrement,                       false, E_CALL,       false, true,  1,   Type::Number,    false) \
    OP(PostfixIncrement,                false, E_CALL,       false, true,  1,   Type::Number,    false) \
    OP(PostfixDecrement,                false, E_CALL,       false, true,  1,   Type::Number,    false) \
    /* String ops */                                                                                                            \
    OP(ConcatString,                    false, E_CALL,       false, true,  2,   Type::String,    false) \
    /* Property access */                                                                                                       \
    OP(GetById,                         false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(GetByIdWithThis,                 false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(GetByValue,                      false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    OP(GetByValueWithThis,              false, E_CALL,       false, true,  3,   Type::Unknown,   false) \
    OP(GetLength,                       false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(PutById,                         false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    OP(PutByIdWithThis,                 false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutByValue,                      false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutByValueWithThis,              false, E_CALL,       false, false, 4,   Type::Unknown,   false) \
    OP(DeleteById,                      false, E_CALL,       false, true,  1,   Type::Boolean,   false) \
    OP(DeleteByIdWithThis,              false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(DeleteByValue,                   false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(DeleteByValueWithThis,           false, E_CALL,       false, true,  3,   Type::Boolean,   false) \
    OP(HasProperty,                     false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(HasPrivateId,                    false, E_CALL,       false, true,  1,   Type::Boolean,   false) \
    OP(GetPrivateById,                  false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(PutPrivateById,                  false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    OP(PutGetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    OP(PutSetterById,                   false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    OP(PutPrototypeById,                false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    OP(PutGetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutSetterByIdWithThis,           false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutPrototypeByIdWithThis,        false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutGetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutSetterByValue,                false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutPrototypeByValue,             false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(PutGetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false) \
    OP(PutSetterByValueWithThis,        false, E_CALL,       false, false, 4,   Type::Unknown,   false) \
    OP(PutPrototypeByValueWithThis,     false, E_CALL,       false, false, 4,   Type::Unknown,   false) \
    OP(PutBySpread,                     false, E_CALL,       false, false, 2,   Type::Unknown,   false) \
    /* Calls */                                                                                                                 \
    OP(Call,                            false, E_CALL,       true,  true,  255, Type::Unknown,   false) \
    OP(CallBuiltin,                     false, E_CALL,       true,  true,  255, Type::Unknown,   false) \
    OP(CallDirectEval,                  false, E_CALL,       true,  true,  255, Type::Unknown,   false) \
    OP(CallWithArgumentArray,           false, E_CALL,       true,  true,  255, Type::Unknown,   false) \
    OP(Construct,                       false, E_CALL,       false, true,  255, Type::Unknown,   false) \
    OP(ConstructWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Unknown,   false) \
    OP(SuperCallWithArgumentArray,      false, E_CALL,       false, true,  255, Type::Unknown,   false) \
    OP(ImportCall,                       false, E_CALL,       false, true,  2,   Type::Unknown,   false) \
    /* Environment */                                                                                                           \
    OP(GetCalleeAndThisFromEnvironment, false, E_CALL,       false, true,  0,   Type::Unknown,   false) \
    OP(CreateVariable,                  false, E_CALL,       false, false, 0,   Type::Unknown,   false) \
    OP(CreateLexicalEnvironment,        false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    OP(CreateMutableBinding,            false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(CreateImmutableBinding,          false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(LeaveLexicalEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(EnterObjectEnvironment,          false, E_CALL,       false, false, 1,   Type::Unknown,   false) \
    OP(GetBinding,                      false, E_CALL,       false, true,  0,   Type::Unknown,   false) \
    OP(InitializeBinding,               false, E_CALL,       false, false, 1,   Type::Unknown,   false) \
    OP(SetBinding,                      false, E_CALL,       false, false, 1,   Type::Unknown,   false) \
    OP(GetGlobal,                       false, E_CALL,       false, true,  0,   Type::Unknown,   false) \
    OP(SetGlobal,                       false, E_CALL,       false, false, 1,   Type::Unknown,   false) \
    OP(DeleteVariable,                  false, E_CALL,       false, true,  0,   Type::Boolean,   false) \
    OP(ResolveThisBinding,              false, E_CALL,       false, false, 0,   Type::Unknown,   false) \
    OP(ResolveSuperBase,                false, E_CALL,       false, true,  0,   Type::Unknown,   false) \
    OP(CreatePrivateEnvironment,        false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(LeavePrivateEnvironment,         false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(AddPrivateName,                  false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(CreateVariableEnvironment,       false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    /* Object creation */                                                                                                       \
    OP(NewObject,                       false, E_THROW,      false, true,  0,   Type::Object,    false) \
    OP(NewArray,                        false, E_THROW,      false, true,  255, Type::Array,     false) \
    OP(NewArrayWithLength,              false, E_THROW,      false, true,  1,   Type::Array,     false) \
    OP(ArrayAppend,                     false, E_THROW_WRITE,false, false, 2,   Type::Unknown,   false) \
    OP(NewClass,                        false, E_CALL,       false, true,  255, Type::Function,  false) \
    OP(NewFunction,                     false, E_THROW,      false, true,  255, Type::Function,  false) \
    OP(NewRegExp,                       false, E_THROW,      false, true,  0,   Type::Object,    false) \
    OP(GetTemplateObject,               false, E_WRITE,      false, true,  255, Type::Array,     false) \
    OP(InitObjectLiteralProperty,       false, E_WRITE,      false, false, 2,   Type::Unknown,   false) \
    OP(CacheObjectShape,                false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(CopyObjectExcludingProperties,   false, E_CALL,       false, true,  255, Type::Unknown,   false) \
    /* Special */                                                                                                               \
    OP(In,                              false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    OP(InstanceOf,                      false, E_CALL,       false, true,  2,   Type::Boolean,   false) \
    /* Iterators */                                                                                                             \
    OP(GetIterator,                     false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(GetObjectPropertyIterator,       false, E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(IteratorNext,                    false, E_CALL,       false, true,  3,   Type::Unknown,   false) \
    OP(IteratorNextUnpack,              false, E_CALL,       false, true,  3,   Type::Unknown,   false) \
    OP(IteratorClose,                   false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(AsyncIteratorClose,              false, E_CALL,       false, false, 3,   Type::Unknown,   false) \
    OP(IteratorToArray,                 false, E_CALL,       false, true,  3,   Type::Array,     false) \
    /* Generators/Async */                                                                                                      \
    OP(Yield,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(Await,                           true,  E_CALL,       false, true,  1,   Type::Unknown,   false) \
    OP(PrepareYield,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(GetCompletionFields,             false, E_THROW,      false, true,  1,   Type::Unknown,   false) \
    OP(SetCompletionType,               false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(NewTypeError,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    /* Copy */                                                                                                                  \
    OP(Move,                            false, E_NONE,       false, true,  1,   Type::Unknown,   false) \
    /* Tuple extraction (for multi-output instructions) */                                                                      \
    OP(ExtractValue,                    false, E_NONE,       false, true,  1,   Type::Unknown,   false) \
    /* Arguments */                                                                                                             \
    OP(CreateArguments,                 false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    OP(CreateRestParams,                false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    /* New target */                                                                                                            \
    OP(GetNewTarget,                    false, E_THROW,      false, true,  0,   Type::Unknown,   false) \
    /* Exception handling */                                                                                                    \
    OP(Catch,                           false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    OP(EnterUnwindContext,              true,  E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(LeaveUnwindContext,              false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(ScheduleJump,                    true,  E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(LeaveFinally,                    false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(RestoreScheduledJump,            false, E_WRITE,      false, false, 0,   Type::Unknown,   false) \
    OP(SetSavedReturnValue,             false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    OP(GetException,                    false, E_WRITE,      false, true,  0,   Type::Unknown,   false) \
    OP(SetException,                    false, E_WRITE,      false, false, 1,   Type::Unknown,   false) \
    /* Guard operations (may throw but produce no value) */                                                                     \
    OP(ThrowIfNotObject,                false, E_THROW,      false, false, 1,   Type::Unknown,   false) \
    OP(ThrowIfNullish,                  false, E_THROW,      false, false, 1,   Type::Unknown,   false) \
    OP(ThrowIfTDZ,                      false, E_THROW,      false, false, 1,   Type::Unknown,   false)
// clang-format on
