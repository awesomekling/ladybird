/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibCore/Platform/MemoryInfo.h>

namespace Core::Platform {

ErrorOr<MemoryInfo> get_memory_info()
{
    return MemoryInfo {};
}

}
