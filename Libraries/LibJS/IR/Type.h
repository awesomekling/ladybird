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

}
