/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Error.h>
#include <LibSandbox/Sandbox.h>

namespace Compositor {

inline Sandbox::Policy sandbox_policy()
{
    static constexpr Sandbox::Capability capabilities[] {
        Sandbox::Capability::FileSystemRead,
        Sandbox::Capability::FileSystemWrite,
        Sandbox::Capability::FileDescriptorIO,
        Sandbox::Capability::LocalIPC,
        Sandbox::Capability::GpuDevice,
        Sandbox::Capability::CommonRuntime,
    };
    static constexpr Sandbox::FileSystemScope filesystem_scopes[] {
        Sandbox::FileSystemScope::Fonts,
        Sandbox::FileSystemScope::GraphicsShaderCache,
    };
    return {
        .capabilities = capabilities,
        .filesystem_scopes = filesystem_scopes,
    };
}

[[nodiscard]] ErrorOr<void> apply_sandbox();

}
