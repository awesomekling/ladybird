/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/ByteString.h>
#include <AK/Error.h>
#include <AK/Vector.h>

namespace Core::Platform {

struct MemoryRegion {
    ByteString name;
    u64 size { 0 };
    u64 resident { 0 };
    u64 dirty { 0 };
};

struct MemoryInfo {
    u64 resident_bytes { 0 };
    u64 phys_footprint { 0 };
    u64 phys_footprint_peak { 0 };
    Vector<MemoryRegion> regions;
};

ErrorOr<MemoryInfo> get_memory_info();

}
