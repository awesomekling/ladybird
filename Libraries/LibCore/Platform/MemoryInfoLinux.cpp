/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Platform.h>

#if !defined(AK_OS_LINUX)
#    error "This file is only available on Linux"
#endif

#include <AK/GenericLexer.h>
#include <AK/LexicalPath.h>
#include <LibCore/File.h>
#include <LibCore/Platform/MemoryInfo.h>

namespace Core::Platform {

static ErrorOr<void> parse_proc_status(MemoryInfo& info)
{
    auto file = TRY(Core::File::open("/proc/self/status"sv, Core::File::OpenMode::Read));
    auto contents = TRY(file->read_until_eof());
    StringView text { contents };

    for (auto line : text.split_view('\n')) {
        GenericLexer lexer { line };

        auto key = lexer.consume_until(':');
        lexer.consume_specific(':');
        lexer.ignore_while(is_ascii_space);
        auto value_str = lexer.consume_until(is_ascii_space);

        auto value = value_str.to_number<u64>();
        if (!value.has_value())
            continue;

        // Values in /proc/self/status are in kB
        u64 bytes = value.value() * 1024;

        if (key == "VmRSS"sv)
            info.resident_bytes = bytes;
        else if (key == "VmPeak"sv)
            info.phys_footprint_peak = bytes;
    }

    // On Linux, phys_footprint is roughly equivalent to VmRSS
    info.phys_footprint = info.resident_bytes;

    return {};
}

static ByteString normalize_region_name(StringView pathname)
{
    // Anonymous mappings
    if (pathname.is_empty())
        return "[anon]";

    // Keep kernel mappings as-is: [heap], [stack], [vdso], [vvar], [vsyscall]
    if (pathname.starts_with('['))
        return ByteString(pathname);

    // For files, just use the basename
    if (pathname.starts_with('/'))
        return ByteString(LexicalPath::basename(pathname));

    return ByteString(pathname);
}

static ErrorOr<void> parse_proc_smaps(MemoryInfo& info)
{
    auto file_or_error = Core::File::open("/proc/self/smaps"sv, Core::File::OpenMode::Read);
    if (file_or_error.is_error())
        return {};

    auto file = file_or_error.release_value();
    auto contents = TRY(file->read_until_eof());
    StringView text { contents };

    MemoryRegion current_region;
    bool has_region = false;
    StringView current_pathname;

    for (auto line : text.split_view('\n')) {
        // Region header lines start with an address range
        if (!line.is_empty() && is_ascii_hex_digit(line[0])) {
            if (has_region && current_region.size > 0) {
                current_region.name = normalize_region_name(current_pathname);
                info.regions.append(move(current_region));
            }

            current_region = {};
            current_pathname = {};
            has_region = true;

            // Extract the region name from the end of the header line
            // Format: address perms offset dev inode pathname
            auto parts = line.split_view(' ', SplitBehavior::KeepEmpty);
            if (parts.size() >= 6) {
                // Find the pathname (last non-empty part)
                for (size_t i = parts.size() - 1; i >= 5 && i < parts.size(); --i) {
                    if (!parts[i].is_empty()) {
                        current_pathname = parts[i];
                        break;
                    }
                }
            }
        } else if (has_region) {
            GenericLexer lexer { line };
            auto key = lexer.consume_until(':');
            lexer.consume_specific(':');
            lexer.ignore_while(is_ascii_space);
            auto value_str = lexer.consume_until(is_ascii_space);

            auto value = value_str.to_number<u64>();
            if (!value.has_value())
                continue;

            // Values in smaps are in kB
            u64 bytes = value.value() * 1024;

            if (key == "Size"sv)
                current_region.size = bytes;
            else if (key == "Rss"sv)
                current_region.resident = bytes;
            else if (key == "Private_Dirty"sv || key == "Shared_Dirty"sv)
                current_region.dirty += bytes;
        }
    }

    if (has_region && current_region.size > 0) {
        current_region.name = normalize_region_name(current_pathname);
        info.regions.append(move(current_region));
    }

    return {};
}

ErrorOr<MemoryInfo> get_memory_info()
{
    MemoryInfo info;

    TRY(parse_proc_status(info));
    TRY(parse_proc_smaps(info));

    return info;
}

}
