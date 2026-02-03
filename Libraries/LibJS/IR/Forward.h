/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>

namespace JS::IR {

class BasicBlock;
class Function;
class Instruction;
class Value;

enum class Opcode : u8;
enum class Type : u8;

}
