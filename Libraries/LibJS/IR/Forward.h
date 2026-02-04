/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/DistinctNumeric.h>
#include <AK/Types.h>

namespace JS::IR {

// Strongly-typed indices to prevent accidental mixing of different index types.
AK_TYPEDEF_DISTINCT_NUMERIC_GENERAL(u32, BlockIndex, Comparison, CastToBool, Increment, CastToUnderlying);
AK_TYPEDEF_DISTINCT_NUMERIC_GENERAL(u32, ValueIndex, Comparison, CastToBool, Increment, CastToUnderlying);

class BasicBlock;
class BinaryOpInstruction;
class BranchInstruction;
class CallInstruction;
class Function;
class GetByIdInstruction;
class Instruction;
class JumpInstruction;
class PhiInstruction;
class TerminatorInstruction;
class UnaryOpInstruction;
class Value;

enum class Opcode : u8;
enum class Type : u8;

}
