/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Passes/CopyCoalescing.h>

namespace JS::IR {

JS_API extern bool g_dump_ir;
JS_API extern bool g_optimize_ir;
JS_API extern bool g_dump_ir_between_passes;
JS_API extern bool g_lower_ir;
JS_API extern bool g_log_tier_ups;
JS_API extern bool g_force_inline;

JS_API CoalescingMap optimize(Function&, SsaConstructionData);

}
