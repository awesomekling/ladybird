/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/ByteString.h>
#include <AK/Error.h>
#include <AK/Vector.h>
#include <LibSandbox/Sandbox.h>

namespace RequestServer {

inline Sandbox::Policy sandbox_policy()
{
    static constexpr Sandbox::Capability capabilities[] {
        Sandbox::Capability::FileSystemRead,
        Sandbox::Capability::FileSystemWrite,
        Sandbox::Capability::FileDescriptorIO,
        Sandbox::Capability::LocalIPC,
        Sandbox::Capability::Network,
        Sandbox::Capability::CommonRuntime,
    };
    static constexpr Sandbox::FileSystemScope filesystem_scopes[] {
        Sandbox::FileSystemScope::HttpCache,
        Sandbox::FileSystemScope::ResolverConfiguration,
        Sandbox::FileSystemScope::TlsCertificates,
    };
    return {
        .capabilities = capabilities,
        .filesystem_scopes = filesystem_scopes,
    };
}

[[nodiscard]] ErrorOr<void> apply_sandbox(Vector<ByteString> const& certificates);

}
