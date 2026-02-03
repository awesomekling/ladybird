/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/StringBuilder.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

JS_API extern bool g_dump_ir;
JS_API extern bool g_optimize_ir;
JS_API extern bool g_dump_ir_between_passes;

JS_API void optimize(Function&);

JS_API void dump(Function const&, StringBuilder&);
JS_API void dump(BasicBlock const&, StringBuilder&);
JS_API void dump(Instruction const&, StringBuilder&);
JS_API void dump(Value const&, StringBuilder&);

JS_API String dump(Function const&);
JS_API String dump(BasicBlock const&);
JS_API String dump(Instruction const&);
JS_API String dump(Value const&);

}
