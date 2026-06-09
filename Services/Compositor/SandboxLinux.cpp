/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <Compositor/Sandbox.h>
#include <LibCore/Directory.h>
#include <LibCore/Environment.h>
#include <LibCore/StandardPaths.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibSandbox/Sandbox.h>
#include <LibSandbox/Seccomp.h>
#include <LibWebView/Utilities.h>

namespace Compositor {

static ErrorOr<void> add_font_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    for (auto const& path : TRY(Gfx::FontDatabase::font_directories()))
        TRY(Sandbox::add_landlock_path_if_exists(paths, path, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, TRY(String::formatted("{}/fonts", WebView::s_ladybird_resource_root)), Sandbox::LandlockPath::Access::ReadOnly));
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

static ErrorOr<void> add_compositor_filesystem_scope(Vector<Sandbox::LandlockPath>& paths, Sandbox::FileSystemScope scope)
{
    switch (scope) {
    case Sandbox::FileSystemScope::Fonts:
        return add_font_read_access(paths);
    case Sandbox::FileSystemScope::GraphicsShaderCache:
        return add_mesa_shader_cache_write_access(paths);
    default:
        VERIFY_NOT_REACHED();
    }
}

ErrorOr<void> apply_sandbox()
{
    TRY(Sandbox::install_no_new_privileges());
    TRY(Sandbox::configure_runtime());

    Vector<Sandbox::LandlockPath> paths;
    auto sandbox_policy = Compositor::sandbox_policy();
    for (auto scope : sandbox_policy.filesystem_scopes)
        TRY(add_compositor_filesystem_scope(paths, scope));

    TRY(Sandbox::restrict_filesystem_with_landlock(paths.span()));

    Sandbox::SeccompPolicy policy;
    Sandbox::allow_capabilities(policy, sandbox_policy.capabilities);
    TRY(policy.install());

    return {};
}

}
