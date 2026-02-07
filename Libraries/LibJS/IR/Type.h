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

// Join two types, producing the most specific type that contains both.
constexpr Type join_types(Type a, Type b)
{
    if (a == b)
        return a;
    // int32 is a subtype of number.
    if ((a == Type::Int32 && b == Type::Number) || (a == Type::Number && b == Type::Int32))
        return Type::Number;
    // array is a subtype of object.
    if ((a == Type::Array && b == Type::Object) || (a == Type::Object && b == Type::Array))
        return Type::Object;
    return Type::Unknown;
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

// Bitset representing a set of concrete types (excluding Unknown).
// One bit per Type value (Undefined=0, Null=1, ..., Array=10).
struct TypeMask {
    u16 bits { 0 };

    static constexpr u16 type_bit(Type type)
    {
        VERIFY(type != Type::Unknown);
        return static_cast<u16>(1 << (static_cast<u8>(type) - 1));
    }

    static constexpr TypeMask for_type(Type type)
    {
        if (type == Type::Unknown)
            return all();
        return { type_bit(type) };
    }

    static constexpr TypeMask all() { return { 0x7FF }; }
    static constexpr TypeMask none() { return { 0 }; }
    static constexpr TypeMask nullish() { return { static_cast<u16>(type_bit(Type::Null) | type_bit(Type::Undefined)) }; }

    constexpr bool is_empty() const { return bits == 0; }
    constexpr bool is_single_type() const { return bits != 0 && (bits & (bits - 1)) == 0; }

    constexpr Type as_type() const
    {
        if (!is_single_type())
            return Type::Unknown;
        // Find which bit is set (1-based index maps to Type enum)
        for (u8 i = 1; i <= 11; ++i) {
            if (bits == (1 << (i - 1)))
                return static_cast<Type>(i);
        }
        VERIFY_NOT_REACHED();
    }

    constexpr bool contains(Type type) const
    {
        if (type == Type::Unknown)
            return true;
        return (bits & type_bit(type)) != 0;
    }

    constexpr bool excludes_nullish() const { return (*this & nullish()).is_empty(); }

    constexpr TypeMask exclude(TypeMask other) const { return *this & ~other; }

    constexpr TypeMask operator|(TypeMask other) const { return { static_cast<u16>(bits | other.bits) }; }
    constexpr TypeMask operator&(TypeMask other) const { return { static_cast<u16>(bits & other.bits) }; }
    constexpr TypeMask operator~() const { return { static_cast<u16>(~bits & all().bits) }; }
    constexpr bool operator==(TypeMask other) const { return bits == other.bits; }
};

}
