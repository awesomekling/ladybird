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
#include <LibJS/IR/Type.h>
#include <LibJS/Runtime/Completion.h>
#include <LibJS/Runtime/Iterator.h>

namespace JS {

class ClassExpression;
class FunctionNode;

}

namespace JS::IR {

enum class [[clang::enum_extensibility(closed)]] Opcode : u8 {
    // Control flow
    Jump,
    Branch,
    Return,
    End,
    Throw,
    ContinuePendingUnwind,

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
    ToNumeric,
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
    PutByIdWithThis,
    PutByValue,
    PutByValueWithThis,
    DeleteById,
    DeleteByIdWithThis,
    DeleteByValue,
    DeleteByValueWithThis,
    HasProperty,
    HasPrivateId,
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
    CreatePrivateEnvironment,
    LeavePrivateEnvironment,
    AddPrivateName,
    CreateVariableEnvironment,

    // Object creation
    NewObject,
    NewArray,
    NewArrayWithLength,
    ArrayAppend,
    NewClass,
    NewFunction,
    NewRegExp,
    GetTemplateObject,
    InitObjectLiteralProperty,
    CacheObjectShape,
    CopyObjectExcludingProperties,

    // Special
    In,
    InstanceOf,

    // Iterators
    GetIterator,
    GetObjectPropertyIterator,
    IteratorNext,
    IteratorNextUnpack,
    IteratorClose,
    AsyncIteratorClose,
    IteratorToArray,

    // Generators/Async
    Yield,
    Await,
    GetCompletionFields,
    SetCompletionType,
    NewTypeError,

    // Copy
    Move,

    // Tuple extraction (for multi-output instructions)
    ExtractValue,

    // Arguments
    CreateArguments,
    CreateRestParams,

    // New target
    GetNewTarget,

    // Exception handling
    Catch,
    EnterUnwindContext,
    LeaveUnwindContext,
    ScheduleJump,
    LeaveFinally,
    RestoreScheduledJump,
    SetSavedReturnValue,
    GetException,
    SetException,

    // Guard operations (may throw but produce no value)
    ThrowIfNotObject,
    ThrowIfNullish,
    ThrowIfTDZ,

    __Count, // Sentinel for static assertions - must be last
};

// Centralized opcode metadata table.
// All opcode properties are defined here to prevent divergence between switches.
struct OpcodeTraits {
    char const* name;
    bool is_terminator;
    bool may_throw;
    bool has_side_effects;
    bool is_pure;
    bool is_hoistable;
    bool is_call_like;
    bool has_result;
    u8 operand_arity;            // Expected operand count (VariableArity = variable)
    Type guaranteed_result_type; // Static result type (Type::Unknown = no guarantee)
};

static constexpr u8 VariableArity = 255;

// clang-format off
static constexpr OpcodeTraits s_opcode_traits[] = {
    // Control flow                                                                                       term   throw  side   pure   hoist  call   result arity type
    [to_underlying(Opcode::Jump)]                               = { "Jump",                               true,  false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::Branch)]                             = { "Branch",                             true,  false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::Return)]                             = { "Return",                             true,  false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::End)]                                = { "End",                                true,  false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::Throw)]                              = { "Throw",                              true,  true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::ContinuePendingUnwind)]              = { "ContinuePendingUnwind",              true,  true,  true,  false, false, false, false, 0,   Type::Unknown   },

    // SSA
    [to_underlying(Opcode::Phi)]                                = { "Phi",                                false, false, false, false, false, false, true,  255, Type::Unknown   },

    // Constants
    [to_underlying(Opcode::LoadConstant)]                       = { "LoadConstant",                       false, false, false, false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::LoadUndefined)]                      = { "LoadUndefined",                      false, false, false, false, false, false, true,  0,   Type::Undefined },
    [to_underlying(Opcode::LoadNull)]                           = { "LoadNull",                           false, false, false, false, false, false, true,  0,   Type::Null      },

    // Arithmetic (may call ToPrimitive on objects)
    [to_underlying(Opcode::Add)]                                = { "Add",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Sub)]                                = { "Sub",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Mul)]                                = { "Mul",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Div)]                                = { "Div",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Mod)]                                = { "Mod",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Exp)]                                = { "Exp",                                false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::Negate)]                             = { "Negate",                             false, true,  true,  false, false, false, true,  1,   Type::Number    },
    [to_underlying(Opcode::UnaryPlus)]                          = { "UnaryPlus",                          false, true,  true,  false, false, false, true,  1,   Type::Number    },

    // Bitwise (may call ToInt32 -> ToPrimitive on objects)
    [to_underlying(Opcode::BitwiseAnd)]                         = { "BitwiseAnd",                         false, true,  true,  false, false, false, true,  2,   Type::Int32     },
    [to_underlying(Opcode::BitwiseOr)]                          = { "BitwiseOr",                          false, true,  true,  false, false, false, true,  2,   Type::Int32     },
    [to_underlying(Opcode::BitwiseXor)]                         = { "BitwiseXor",                         false, true,  true,  false, false, false, true,  2,   Type::Int32     },
    [to_underlying(Opcode::BitwiseNot)]                         = { "BitwiseNot",                         false, true,  true,  false, false, false, true,  1,   Type::Int32     },
    [to_underlying(Opcode::LeftShift)]                          = { "LeftShift",                          false, true,  true,  false, false, false, true,  2,   Type::Int32     },
    [to_underlying(Opcode::RightShift)]                         = { "RightShift",                         false, true,  true,  false, false, false, true,  2,   Type::Int32     },
    [to_underlying(Opcode::UnsignedRightShift)]                 = { "UnsignedRightShift",                 false, true,  true,  false, false, false, true,  2,   Type::Number    },

    // Comparison (relational may call ToPrimitive, loose equality too)
    [to_underlying(Opcode::LessThan)]                           = { "LessThan",                           false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::LessThanEquals)]                     = { "LessThanEquals",                     false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::GreaterThan)]                        = { "GreaterThan",                        false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::GreaterThanEquals)]                  = { "GreaterThanEquals",                  false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::LooselyEquals)]                      = { "LooselyEquals",                      false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::StrictlyEquals)]                     = { "StrictlyEquals",                     false, false, false, true,  false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::LooselyInequals)]                    = { "LooselyInequals",                    false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::StrictlyInequals)]                   = { "StrictlyInequals",                   false, false, false, true,  false, false, true,  2,   Type::Boolean   },

    // Type ops
    [to_underlying(Opcode::Typeof)]                             = { "Typeof",                             false, false, false, true,  true,  false, true,  1,   Type::String    },
    [to_underlying(Opcode::TypeofBinding)]                      = { "TypeofBinding",                      false, true,  true,  false, false, false, true,  0,   Type::String    },
    [to_underlying(Opcode::ToBoolean)]                          = { "ToBoolean",                          false, false, false, true,  true,  false, true,  1,   Type::Boolean   },
    [to_underlying(Opcode::ToNumber)]                           = { "ToNumber",                           false, true,  true,  false, false, false, true,  1,   Type::Number    },
    [to_underlying(Opcode::ToNumeric)]                          = { "ToNumeric",                          false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::ToString)]                           = { "ToString",                           false, true,  true,  false, false, false, true,  1,   Type::String    },
    [to_underlying(Opcode::ToObject)]                           = { "ToObject",                           false, true,  true,  false, false, false, true,  1,   Type::Object    },
    [to_underlying(Opcode::ToInt32)]                            = { "ToInt32",                            false, true,  true,  false, false, false, true,  1,   Type::Int32     },
    [to_underlying(Opcode::ToLength)]                           = { "ToLength",                           false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::Not)]                                = { "Not",                                false, false, false, true,  true,  false, true,  1,   Type::Boolean   },
    [to_underlying(Opcode::IsUndefined)]                        = { "IsUndefined",                        false, false, false, true,  true,  false, true,  1,   Type::Boolean   },
    [to_underlying(Opcode::IsNullish)]                          = { "IsNullish",                          false, false, false, true,  true,  false, true,  1,   Type::Boolean   },

    // Increment/Decrement (may call ToNumber -> ToPrimitive)
    [to_underlying(Opcode::Increment)]                          = { "Increment",                          false, true,  true,  false, false, false, true,  1,   Type::Number    },
    [to_underlying(Opcode::Decrement)]                          = { "Decrement",                          false, true,  true,  false, false, false, true,  1,   Type::Number    },
    [to_underlying(Opcode::PostfixIncrement)]                   = { "PostfixIncrement",                   false, true,  true,  false, false, false, true,  1,   Type::Number    },
    [to_underlying(Opcode::PostfixDecrement)]                   = { "PostfixDecrement",                   false, true,  true,  false, false, false, true,  1,   Type::Number    },

    // String ops
    [to_underlying(Opcode::ConcatString)]                       = { "ConcatString",                       false, true,  true,  false, false, false, true,  2,   Type::String    },

    // Property access
    [to_underlying(Opcode::GetById)]                            = { "GetById",                            false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::GetByIdWithThis)]                    = { "GetByIdWithThis",                    false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::GetByValue)]                         = { "GetByValue",                         false, true,  true,  false, false, false, true,  2,   Type::Unknown   },
    [to_underlying(Opcode::GetByValueWithThis)]                 = { "GetByValueWithThis",                 false, true,  true,  false, false, false, true,  3,   Type::Unknown   },
    [to_underlying(Opcode::GetLength)]                          = { "GetLength",                          false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::PutById)]                            = { "PutById",                            false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::PutByIdWithThis)]                    = { "PutByIdWithThis",                    false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutByValue)]                         = { "PutByValue",                         false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutByValueWithThis)]                 = { "PutByValueWithThis",                 false, true,  true,  false, false, false, false, 4,   Type::Unknown   },
    [to_underlying(Opcode::DeleteById)]                         = { "DeleteById",                         false, true,  true,  false, false, false, true,  1,   Type::Boolean   },
    [to_underlying(Opcode::DeleteByIdWithThis)]                 = { "DeleteByIdWithThis",                 false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::DeleteByValue)]                      = { "DeleteByValue",                      false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::DeleteByValueWithThis)]              = { "DeleteByValueWithThis",              false, true,  true,  false, false, false, true,  3,   Type::Boolean   },
    [to_underlying(Opcode::HasProperty)]                        = { "HasProperty",                        false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::HasPrivateId)]                       = { "HasPrivateId",                       false, true,  true,  false, false, false, true,  1,   Type::Boolean   },
    [to_underlying(Opcode::GetPrivateById)]                     = { "GetPrivateById",                     false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::PutPrivateById)]                     = { "PutPrivateById",                     false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::PutGetterById)]                      = { "PutGetterById",                      false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::PutSetterById)]                      = { "PutSetterById",                      false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::PutPrototypeById)]                   = { "PutPrototypeById",                   false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::PutGetterByIdWithThis)]              = { "PutGetterByIdWithThis",              false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutSetterByIdWithThis)]              = { "PutSetterByIdWithThis",              false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutPrototypeByIdWithThis)]           = { "PutPrototypeByIdWithThis",           false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutGetterByValue)]                   = { "PutGetterByValue",                   false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutSetterByValue)]                   = { "PutSetterByValue",                   false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutPrototypeByValue)]                = { "PutPrototypeByValue",                false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::PutGetterByValueWithThis)]           = { "PutGetterByValueWithThis",           false, true,  true,  false, false, false, false, 4,   Type::Unknown   },
    [to_underlying(Opcode::PutSetterByValueWithThis)]           = { "PutSetterByValueWithThis",           false, true,  true,  false, false, false, false, 4,   Type::Unknown   },
    [to_underlying(Opcode::PutPrototypeByValueWithThis)]        = { "PutPrototypeByValueWithThis",        false, true,  true,  false, false, false, false, 4,   Type::Unknown   },
    [to_underlying(Opcode::PutBySpread)]                        = { "PutBySpread",                        false, true,  true,  false, false, false, false, 2,   Type::Unknown   },

    // Calls
    [to_underlying(Opcode::Call)]                               = { "Call",                               false, true,  true,  false, false, true,  true,  255, Type::Unknown   },
    [to_underlying(Opcode::CallBuiltin)]                        = { "CallBuiltin",                        false, true,  true,  false, false, true,  true,  255, Type::Unknown   },
    [to_underlying(Opcode::CallDirectEval)]                     = { "CallDirectEval",                     false, true,  true,  false, false, true,  true,  255, Type::Unknown   },
    [to_underlying(Opcode::CallWithArgumentArray)]              = { "CallWithArgumentArray",              false, true,  true,  false, false, true,  true,  255, Type::Unknown   },
    [to_underlying(Opcode::Construct)]                          = { "Construct",                          false, true,  true,  false, false, false, true,  255, Type::Unknown   },
    [to_underlying(Opcode::ConstructWithArgumentArray)]         = { "ConstructWithArgumentArray",         false, true,  true,  false, false, false, true,  255, Type::Unknown   },
    [to_underlying(Opcode::SuperCallWithArgumentArray)]         = { "SuperCallWithArgumentArray",         false, true,  true,  false, false, false, true,  255, Type::Unknown   },
    [to_underlying(Opcode::ImportCall)]                         = { "ImportCall",                         false, true,  true,  false, false, false, true,  2,   Type::Unknown   },

    // Environment
    [to_underlying(Opcode::GetCalleeAndThisFromEnvironment)]    = { "GetCalleeAndThisFromEnvironment",    false, true,  true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::CreateVariable)]                     = { "CreateVariable",                     false, true,  true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::CreateLexicalEnvironment)]           = { "CreateLexicalEnvironment",           false, false, true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::CreateMutableBinding)]               = { "CreateMutableBinding",               false, false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::CreateImmutableBinding)]             = { "CreateImmutableBinding",             false, false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::LeaveLexicalEnvironment)]            = { "LeaveLexicalEnvironment",            false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::EnterObjectEnvironment)]             = { "EnterObjectEnvironment",             false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::GetBinding)]                         = { "GetBinding",                         false, true,  true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::InitializeBinding)]                  = { "InitializeBinding",                  false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::SetBinding)]                         = { "SetBinding",                         false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::GetGlobal)]                          = { "GetGlobal",                          false, true,  true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::SetGlobal)]                          = { "SetGlobal",                          false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::DeleteVariable)]                     = { "DeleteVariable",                     false, true,  true,  false, false, false, true,  0,   Type::Boolean   },
    [to_underlying(Opcode::ResolveThisBinding)]                 = { "ResolveThisBinding",                 false, true,  true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::ResolveSuperBase)]                   = { "ResolveSuperBase",                   false, true,  true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::CreatePrivateEnvironment)]           = { "CreatePrivateEnvironment",           false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::LeavePrivateEnvironment)]            = { "LeavePrivateEnvironment",            false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::AddPrivateName)]                     = { "AddPrivateName",                     false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::CreateVariableEnvironment)]          = { "CreateVariableEnvironment",          false, false, true,  false, false, false, false, 0,   Type::Unknown   },

    // Object creation
    [to_underlying(Opcode::NewObject)]                          = { "NewObject",                          false, true,  true,  false, false, false, true,  0,   Type::Object    },
    [to_underlying(Opcode::NewArray)]                           = { "NewArray",                           false, true,  true,  false, false, false, true,  255, Type::Array     },
    [to_underlying(Opcode::NewArrayWithLength)]                 = { "NewArrayWithLength",                 false, true,  true,  false, false, false, true,  1,   Type::Array     },
    [to_underlying(Opcode::ArrayAppend)]                        = { "ArrayAppend",                        false, true,  true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::NewClass)]                           = { "NewClass",                           false, true,  true,  false, false, false, true,  255, Type::Function  },
    [to_underlying(Opcode::NewFunction)]                        = { "NewFunction",                        false, true,  true,  false, false, false, true,  255, Type::Function  },
    [to_underlying(Opcode::NewRegExp)]                          = { "NewRegExp",                          false, true,  true,  false, false, false, true,  0,   Type::Object    },
    [to_underlying(Opcode::GetTemplateObject)]                  = { "GetTemplateObject",                  false, false, true,  false, false, false, true,  255, Type::Array     },
    [to_underlying(Opcode::InitObjectLiteralProperty)]          = { "InitObjectLiteralProperty",          false, false, true,  false, false, false, false, 2,   Type::Unknown   },
    [to_underlying(Opcode::CacheObjectShape)]                   = { "CacheObjectShape",                   false, false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::CopyObjectExcludingProperties)]      = { "CopyObjectExcludingProperties",      false, true,  true,  false, false, false, true,  255, Type::Unknown   },

    // Special
    [to_underlying(Opcode::In)]                                 = { "In",                                 false, true,  true,  false, false, false, true,  2,   Type::Boolean   },
    [to_underlying(Opcode::InstanceOf)]                         = { "InstanceOf",                         false, true,  true,  false, false, false, true,  2,   Type::Boolean   },

    // Iterators
    [to_underlying(Opcode::GetIterator)]                        = { "GetIterator",                        false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::GetObjectPropertyIterator)]          = { "GetObjectPropertyIterator",          false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::IteratorNext)]                       = { "IteratorNext",                       false, true,  true,  false, false, false, true,  3,   Type::Unknown   },
    [to_underlying(Opcode::IteratorNextUnpack)]                 = { "IteratorNextUnpack",                 false, true,  true,  false, false, false, true,  3,   Type::Unknown   },
    [to_underlying(Opcode::IteratorClose)]                      = { "IteratorClose",                      false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::AsyncIteratorClose)]                 = { "AsyncIteratorClose",                 false, true,  true,  false, false, false, false, 3,   Type::Unknown   },
    [to_underlying(Opcode::IteratorToArray)]                    = { "IteratorToArray",                    false, true,  true,  false, false, false, true,  3,   Type::Array     },

    // Generators/Async
    [to_underlying(Opcode::Yield)]                              = { "Yield",                              true,  true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::Await)]                              = { "Await",                              true,  true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::GetCompletionFields)]                = { "GetCompletionFields",                false, true,  true,  false, false, false, true,  1,   Type::Unknown   },
    [to_underlying(Opcode::SetCompletionType)]                  = { "SetCompletionType",                  false, false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::NewTypeError)]                       = { "NewTypeError",                       false, false, true,  false, false, false, true,  0,   Type::Unknown   },

    // Copy
    [to_underlying(Opcode::Move)]                               = { "Move",                               false, false, false, false, false, false, true,  1,   Type::Unknown   },

    // Tuple extraction
    [to_underlying(Opcode::ExtractValue)]                       = { "ExtractValue",                       false, false, false, false, false, false, true,  1,   Type::Unknown   },

    // Arguments
    [to_underlying(Opcode::CreateArguments)]                    = { "CreateArguments",                    false, false, true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::CreateRestParams)]                   = { "CreateRestParams",                   false, false, true,  false, false, false, true,  0,   Type::Unknown   },

    // New target
    [to_underlying(Opcode::GetNewTarget)]                       = { "GetNewTarget",                       false, true,  true,  false, false, false, true,  0,   Type::Unknown   },

    // Exception handling
    [to_underlying(Opcode::Catch)]                              = { "Catch",                              false, false, true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::EnterUnwindContext)]                  = { "EnterUnwindContext",                  true,  false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::LeaveUnwindContext)]                  = { "LeaveUnwindContext",                  false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::ScheduleJump)]                       = { "ScheduleJump",                       true,  false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::LeaveFinally)]                       = { "LeaveFinally",                       false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::RestoreScheduledJump)]               = { "RestoreScheduledJump",               false, false, true,  false, false, false, false, 0,   Type::Unknown   },
    [to_underlying(Opcode::SetSavedReturnValue)]                = { "SetSavedReturnValue",                false, false, true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::GetException)]                       = { "GetException",                       false, false, true,  false, false, false, true,  0,   Type::Unknown   },
    [to_underlying(Opcode::SetException)]                       = { "SetException",                       false, false, true,  false, false, false, false, 1,   Type::Unknown   },

    // Guard operations (may throw but produce no value)
    [to_underlying(Opcode::ThrowIfNotObject)]                   = { "ThrowIfNotObject",                   false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::ThrowIfNullish)]                     = { "ThrowIfNullish",                     false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
    [to_underlying(Opcode::ThrowIfTDZ)]                         = { "ThrowIfTDZ",                         false, true,  true,  false, false, false, false, 1,   Type::Unknown   },
};
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

constexpr bool may_throw_opcode(Opcode opcode)
{
    return opcode_traits(opcode).may_throw;
}

// Does this opcode have observable side effects at the opcode level?
// NB: This is conservative - arithmetic/comparison ops CAN have side effects
// when operands are objects (ToPrimitive calls valueOf/toString).
// Use Instruction::has_side_effects() for type-aware analysis.
constexpr bool has_side_effects_opcode(Opcode opcode)
{
    return opcode_traits(opcode).has_side_effects;
}

// Is this opcode pure (no side effects, deterministic) at the opcode level?
// NB: This is conservative - arithmetic/comparison ops are NOT pure when
// operands could be objects. Use Instruction::is_pure() for type-aware analysis.
constexpr bool is_pure_opcode(Opcode opcode)
{
    return opcode_traits(opcode).is_pure;
}

// Can this instruction be safely hoisted out of a loop at the opcode level?
// Must be pure AND not throw. NB: This is conservative - arithmetic ops can
// call ToPrimitive on objects. Use Instruction::is_hoistable() for type-aware analysis.
constexpr bool is_hoistable_opcode(Opcode opcode)
{
    return opcode_traits(opcode).is_hoistable;
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
    bool may_throw() const { return may_throw_opcode(m_opcode); }

    // Re-derive result type from current operand types.
    // Called during initial construction and after Phi type resolution.
    void recompute_result_type();

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

    BasicBlock* true_target() const { return m_true_target; }
    BasicBlock* false_target() const { return m_false_target; }

protected:
    explicit TerminatorInstruction(Opcode opcode);

private:
    friend class CFG;
    friend class Function;
    friend class JumpInstruction;
    friend class BranchInstruction;

    // All target mutation goes through CFG helpers or subclass-specific APIs.
    void set_true_target(BasicBlock* block) { m_true_target = block; }
    void set_false_target(BasicBlock* block) { m_false_target = block; }

    BasicBlock* m_true_target { nullptr };
    BasicBlock* m_false_target { nullptr };
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

    void set_target(BasicBlock& block) { set_true_target(&block); }

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

private:
    friend class CFG;

    void set_true_branch(BasicBlock& block) { set_true_target(&block); }
    void set_false_branch(BasicBlock& block) { set_false_target(&block); }

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
    BasicBlock* incoming_block(size_t index) const { return phi_predecessors()[index]; }
    Value* incoming_value(size_t index) const { return operands()[index]; }

    // Atomic modification methods - these keep predecessor/value pairs in sync
    void add_incoming(BasicBlock* predecessor, Value* value);
    void remove_incoming(size_t index);
    void remove_incoming_from(BasicBlock* predecessor);
    void set_incoming_block(size_t index, BasicBlock* block);
    void set_incoming_value(size_t index, Value* value);

private:
    PhiInstruction();

    // Hide dangerous base class methods that could desync predecessor/value pairs
    using Instruction::add_phi_operand;
    using Instruction::remove_phi_operand;
    using Instruction::set_phi_predecessor;

    // Classes that need internal access to phi operations
    friend class Lifter;
    friend class Function;
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
    [[nodiscard]] static NonnullOwnPtr<GetByIdInstruction> create(Value* base, Bytecode::PropertyKeyTableIndex property);

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
    [[nodiscard]] static NonnullOwnPtr<BinaryOpInstruction> create(Opcode opcode, Value* lhs, Value* rhs);

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
    [[nodiscard]] static NonnullOwnPtr<UnaryOpInstruction> create(Opcode opcode, Value* operand);

    Value* operand() const
    {
        VERIFY(operands().size() == 1);
        return operands()[0];
    }

private:
    UnaryOpInstruction(Opcode opcode, Value* operand);
};

}
