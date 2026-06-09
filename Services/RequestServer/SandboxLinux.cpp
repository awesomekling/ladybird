/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <AK/LexicalPath.h>
#include <AK/String.h>
#include <LibCore/Directory.h>
#include <LibCore/StandardPaths.h>
#include <LibSandbox/Sandbox.h>
#include <LibSandbox/Seccomp.h>
#include <RequestServer/Sandbox.h>

namespace RequestServer {

static ErrorOr<void> add_http_cache_write_access(Vector<Sandbox::LandlockPath>& paths)
{
    auto cache_path = TRY(String::formatted("{}/Ladybird", Core::StandardPaths::cache_directory()));
    TRY(Core::Directory::create(cache_path.to_byte_string(), Core::Directory::CreateDirectories::Yes));
    TRY(Sandbox::add_landlock_path_if_exists(paths, cache_path, Sandbox::LandlockPath::Access::ReadWrite));
    return {};
}

static ErrorOr<void> add_resolver_configuration_read_access(Vector<Sandbox::LandlockPath>& paths)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/host.conf"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/hosts"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/nsswitch.conf"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/resolv.conf"sv, Sandbox::LandlockPath::Access::ReadOnly));
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/run/systemd/resolve"sv, Sandbox::LandlockPath::Access::ReadOnly));
    return {};
}

static ErrorOr<void> add_tls_certificate_read_access(Vector<Sandbox::LandlockPath>& paths, Vector<ByteString> const& certificates)
{
    TRY(Sandbox::add_landlock_path_if_exists(paths, "/etc/ssl"sv, Sandbox::LandlockPath::Access::ReadOnly));

    for (auto const& certificate : certificates) {
        auto certificate_path = LexicalPath::dirname(certificate);
        if (certificate_path.is_empty())
            certificate_path = ".";

        TRY(Sandbox::add_landlock_path_if_exists(paths, certificate_path, Sandbox::LandlockPath::Access::ReadOnly));
    }

    return {};
}

static ErrorOr<void> add_request_server_filesystem_scope(Vector<Sandbox::LandlockPath>& paths, Sandbox::FileSystemScope scope, Vector<ByteString> const& certificates)
{
    switch (scope) {
    case Sandbox::FileSystemScope::HttpCache:
        return add_http_cache_write_access(paths);
    case Sandbox::FileSystemScope::ResolverConfiguration:
        return add_resolver_configuration_read_access(paths);
    case Sandbox::FileSystemScope::TlsCertificates:
        return add_tls_certificate_read_access(paths, certificates);
    default:
        VERIFY_NOT_REACHED();
    }
}

ErrorOr<void> apply_sandbox(Vector<ByteString> const& certificates)
{
    TRY(Sandbox::install_no_new_privileges());
    TRY(Sandbox::configure_runtime());

    Vector<Sandbox::LandlockPath> paths;
    auto sandbox_policy = RequestServer::sandbox_policy();
    for (auto scope : sandbox_policy.filesystem_scopes)
        TRY(add_request_server_filesystem_scope(paths, scope, certificates));

    TRY(Sandbox::restrict_filesystem_with_landlock(paths.span()));

    Sandbox::SeccompPolicy policy;
    Sandbox::allow_capabilities(policy, sandbox_policy.capabilities);
    TRY(policy.install());

    return {};
}

}
