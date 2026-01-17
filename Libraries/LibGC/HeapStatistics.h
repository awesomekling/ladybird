/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>

namespace GC {

struct HeapStatistics {
    size_t total_allocated_bytes { 0 };
    size_t total_live_cell_bytes { 0 };
    size_t block_count { 0 };
};

}
