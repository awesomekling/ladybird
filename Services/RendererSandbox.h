/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Error.h>
#include <AK/Optional.h>
#include <AK/StringView.h>
#include <LibSandbox/Sandbox.h>

namespace RendererSandbox {

inline Sandbox::Policy sandbox_policy()
{
    static constexpr Sandbox::Capability capabilities[] {
        Sandbox::Capability::FileSystemRead,
        Sandbox::Capability::FileSystemWrite,
        Sandbox::Capability::FileDescriptorIO,
        Sandbox::Capability::ProcessCreation,
        Sandbox::Capability::LocalIPC,
        Sandbox::Capability::GpuDevice,
        Sandbox::Capability::CommonRuntime,
        Sandbox::Capability::ExecutableMemory,
    };
    static constexpr Sandbox::FileSystemScope filesystem_scopes[] {
        Sandbox::FileSystemScope::BrowserResources,
        Sandbox::FileSystemScope::Configuration,
        Sandbox::FileSystemScope::CrashSymbolication,
        Sandbox::FileSystemScope::SharedLibraries,
        Sandbox::FileSystemScope::GraphicsDriverResources,
        Sandbox::FileSystemScope::GraphicsDeviceAccess,
        Sandbox::FileSystemScope::HardwareMetadata,
        Sandbox::FileSystemScope::Fonts,
        Sandbox::FileSystemScope::WasmCompiler,
        Sandbox::FileSystemScope::GraphicsShaderCache,
        Sandbox::FileSystemScope::AudioRuntime,
    };
    return {
        .capabilities = capabilities,
        .filesystem_scopes = filesystem_scopes,
    };
}

[[nodiscard]] ErrorOr<void> apply_sandbox(Optional<StringView> config_path);

}
