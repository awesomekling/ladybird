/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <AK/LexicalPath.h>
#include <LibCore/Directory.h>
#include <LibCore/Environment.h>
#include <LibCore/StandardPaths.h>
#include <LibCore/System.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibSandbox/Sandbox.h>
#include <LibSandbox/Seccomp.h>
#include <LibWebView/Utilities.h>
#include <Services/RendererSandbox.h>

namespace RendererSandbox {

struct FileSystemContext {
    Optional<StringView> config_path;
    ByteString executable_path;
    ByteString build_root;
};

static ErrorOr<void> add_browser_resource_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, WebView::s_ladybird_resource_root, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_configuration_read_access(Vector<Sandbox::LandlockPath>& paths, FileSystemContext const& context)
{
    if (context.config_path.has_value())
        TRY(Sandbox::add_landlock_path_if_exists(paths, *context.config_path, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_crash_symbolication_read_access(Vector<Sandbox::LandlockPath>& paths, FileSystemContext const& context)
{
    // cpptrace opens loaded ELF objects when symbolizing in-process stack traces.
    TRY(Sandbox::add_landlock_path_if_exists(paths, context.executable_path, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, LexicalPath::join(context.build_root, "lib"sv).string(), Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_shared_library_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/proc/self"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/lib"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/lib64"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/usr/lib"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/usr/local/lib"sv, Sandbox::LandlockPath::Access::ReadOnly));
    if (auto library_path = Core::Environment::get("LD_LIBRARY_PATH"sv); library_path.has_value()) {
        for (auto path : library_path->split_view(':'))
            TRY(Sandbox::add_landlock_path_if_exists(paths, path, Sandbox::LandlockPath::Access::ReadOnly));
    }
    return {};
}

static ErrorOr<void> add_graphics_driver_resource_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/glvnd"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/usr/share/glvnd"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/usr/share/drirc.d"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/usr/share/vulkan"sv, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_gpu_device_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/dev/dri"sv, Sandbox::LandlockPath::Access::ReadWrite));
    return {};
}

static ErrorOr<void> add_sysfs_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/sys"sv, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_font_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    for (auto const& path : TRY(Gfx::FontDatabase::font_directories()))
        TRY(Sandbox::add_landlock_path_if_exists(paths, path, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_wasm_compiler_execute_access(Vector<Sandbox::LandlockPath>& paths, FileSystemContext const& context)
{
    if (auto cranelift_compiler_path = Core::Environment::get("LADYBIRD_CRANELIFT_COMPILER"sv); cranelift_compiler_path.has_value()) {
        TRY(Sandbox::add_landlock_path_if_exists(paths, *cranelift_compiler_path, Sandbox::LandlockPath::Access::ReadAndExecute));
    } else {
        auto default_cranelift_compiler_path = LexicalPath::join(context.build_root, "bin/cranelift-compiler"sv).string();
        TRY(Sandbox::add_landlock_path_if_exists(paths, default_cranelift_compiler_path, Sandbox::LandlockPath::Access::ReadAndExecute));
    }
    return {};
}

static ErrorOr<void> add_mesa_shader_cache_write_access(Vector<Sandbox::LandlockPath>& paths)
{
    auto mesa_shader_cache_path = Core::Environment::get("MESA_SHADER_CACHE_DIR"sv)
                                      .map([](auto path) { return path.to_byte_string(); })
                                      .value_or_lazy_evaluated([] { return ByteString::formatted("{}/mesa_shader_cache", Core::StandardPaths::cache_directory()); });
    TRY(Core::Directory::create(mesa_shader_cache_path, Core::Directory::CreateDirectories::Yes));
    TRY(Sandbox::add_landlock_path_if_exists(paths, mesa_shader_cache_path, Sandbox::LandlockPath::Access::ReadWrite));
    return {};
}

static ErrorOr<void> add_audio_runtime_access(Vector<Sandbox::LandlockPath>& paths)
{
    auto pulse_runtime_path = LexicalPath::join(TRY(Core::StandardPaths::runtime_directory()), "pulse"sv).string();
    TRY(Core::Directory::create(pulse_runtime_path, Core::Directory::CreateDirectories::Yes, 0700));
    TRY(Sandbox::add_landlock_path_if_exists(paths, pulse_runtime_path, Sandbox::LandlockPath::Access::ReadWrite));
    TRY(Sandbox::add_landlock_path_if_exists(paths, LexicalPath::join(Core::StandardPaths::config_directory(), "pulse"sv).string(), Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_renderer_filesystem_scope(Vector<Sandbox::LandlockPath>& paths, Sandbox::FileSystemScope scope, FileSystemContext const& context)
{
    switch (scope) {
    case Sandbox::FileSystemScope::BrowserResources:
        return add_browser_resource_read_access(paths);
    case Sandbox::FileSystemScope::Configuration:
        return add_configuration_read_access(paths, context);
    case Sandbox::FileSystemScope::CrashSymbolication:
        return add_crash_symbolication_read_access(paths, context);
    case Sandbox::FileSystemScope::SharedLibraries:
        return add_shared_library_read_access(paths);
    case Sandbox::FileSystemScope::GraphicsDriverResources:
        return add_graphics_driver_resource_read_access(paths);
    case Sandbox::FileSystemScope::GraphicsDeviceAccess:
        return add_gpu_device_access(paths);
    case Sandbox::FileSystemScope::HardwareMetadata:
        return add_sysfs_read_access(paths);
    case Sandbox::FileSystemScope::Fonts:
        return add_font_read_access(paths);
    case Sandbox::FileSystemScope::WasmCompiler:
        return add_wasm_compiler_execute_access(paths, context);
    case Sandbox::FileSystemScope::GraphicsShaderCache:
        return add_mesa_shader_cache_write_access(paths);
    case Sandbox::FileSystemScope::AudioRuntime:
        return add_audio_runtime_access(paths);
    default:
        VERIFY_NOT_REACHED();
    }
}

ErrorOr<void> apply_sandbox(Optional<StringView> config_path)
{
    TRY(Sandbox::install_no_new_privileges());
    TRY(Sandbox::configure_runtime());

    auto executable_path = TRY(Core::System::current_executable_path());
    auto build_root = LexicalPath::dirname(LexicalPath::dirname(executable_path));
    FileSystemContext filesystem_context {
        .config_path = config_path,
        .executable_path = executable_path,
        .build_root = build_root,
    };

    Vector<Sandbox::LandlockPath> paths;
    auto sandbox_policy = RendererSandbox::sandbox_policy();
    for (auto scope : sandbox_policy.filesystem_scopes)
        TRY(add_renderer_filesystem_scope(paths, scope, filesystem_context));

    TRY(Sandbox::restrict_filesystem_with_landlock(paths.span()));

    Sandbox::SeccompPolicy policy;
    Sandbox::allow_capabilities(policy, sandbox_policy.capabilities);
    TRY(policy.install());

    return {};
}

}
