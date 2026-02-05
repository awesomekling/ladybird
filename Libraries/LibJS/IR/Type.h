/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Assertions.h>
#include <AK/Types.h>

namespace JS::IR {

enum class Type : u8 {
    Unknown,
    Undefined,
    Null,
    Boolean,
    Int32,
    Number,
    String,
    Symbol,
    BigInt,
    Object,
    Function,
    Array,
};

constexpr char const* type_to_string(Type type)
{
    switch (type) {
    case Type::Unknown:
        return "unknown";
    case Type::Undefined:
        return "undefined";
    case Type::Null:
        return "null";
    case Type::Boolean:
        return "boolean";
    case Type::Int32:
        return "int32";
    case Type::Number:
        return "number";
    case Type::String:
        return "string";
    case Type::Symbol:
        return "symbol";
    case Type::BigInt:
        return "bigint";
    case Type::Object:
        return "object";
    case Type::Function:
        return "function";
    case Type::Array:
        return "array";
    }
    VERIFY_NOT_REACHED();
}

// Is this type a primitive that won't trigger ToPrimitive/valueOf calls?
// Used to determine if arithmetic/comparison operations are safe from user code.
constexpr bool is_safe_primitive_type(Type type)
{
    switch (type) {
    case Type::Undefined:
    case Type::Null:
    case Type::Boolean:
    case Type::Int32:
    case Type::Number:
    case Type::String:
        return true;
    // BigInt and Symbol don't call user code but can throw on coercion
    case Type::BigInt:
    case Type::Symbol:
    // Object types can trigger ToPrimitive which calls valueOf/toString
    case Type::Object:
    case Type::Function:
    case Type::Array:
    // Unknown could be anything
    case Type::Unknown:
        return false;
    }
    VERIFY_NOT_REACHED();
}

// Is this type safe for numeric operations (no throws, no user code)?
// More restrictive than is_safe_primitive_type - excludes String (+ is concat)
// and BigInt/Symbol (can throw on mixed operations).
constexpr bool is_safe_numeric_type(Type type)
{
    switch (type) {
    case Type::Undefined:
    case Type::Null:
    case Type::Boolean:
    case Type::Int32:
    case Type::Number:
        return true;
    default:
        return false;
    }
}

}
