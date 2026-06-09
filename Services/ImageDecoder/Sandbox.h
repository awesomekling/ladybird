/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Error.h>
#include <LibSandbox/Sandbox.h>

namespace ImageDecoder {

inline Sandbox::Policy sandbox_policy()
{
    static constexpr Sandbox::Capability capabilities[] {
        Sandbox::Capability::DenyFileSystemProbes,
        Sandbox::Capability::FileDescriptorIO,
        Sandbox::Capability::LocalIPC,
        Sandbox::Capability::CommonRuntime,
    };
    return {
        .capabilities = capabilities,
        .filesystem_scopes = {},
    };
}

[[nodiscard]] ErrorOr<void> apply_sandbox();

}
