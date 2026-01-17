/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Platform.h>

#if !defined(AK_OS_MACH)
#    error "This file is only available on Mach platforms"
#endif

#include <LibCore/MachPort.h>
#include <LibCore/Platform/MemoryInfo.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <mach/vm_statistics.h>

namespace Core::Platform {

static char const* vm_tag_to_name(int tag)
{
    switch (tag) {
    case VM_MEMORY_MALLOC:
        return "MALLOC";
    case VM_MEMORY_MALLOC_SMALL:
        return "MALLOC_SMALL";
    case VM_MEMORY_MALLOC_LARGE:
        return "MALLOC_LARGE";
    case VM_MEMORY_MALLOC_HUGE:
        return "MALLOC_HUGE";
    case VM_MEMORY_SBRK:
        return "SBRK";
    case VM_MEMORY_REALLOC:
        return "REALLOC";
    case VM_MEMORY_MALLOC_TINY:
        return "MALLOC_TINY";
    case VM_MEMORY_MALLOC_LARGE_REUSABLE:
        return "MALLOC_LARGE_REUSABLE";
    case VM_MEMORY_MALLOC_LARGE_REUSED:
        return "MALLOC_LARGE_REUSED";
    case VM_MEMORY_ANALYSIS_TOOL:
        return "ANALYSIS_TOOL";
    case VM_MEMORY_MALLOC_NANO:
        return "MALLOC_NANO";
    case VM_MEMORY_MALLOC_MEDIUM:
        return "MALLOC_MEDIUM";
    case VM_MEMORY_MACH_MSG:
        return "MACH_MSG";
    case VM_MEMORY_IOKIT:
        return "IOKIT";
    case VM_MEMORY_STACK:
        return "STACK";
    case VM_MEMORY_GUARD:
        return "GUARD";
    case VM_MEMORY_SHARED_PMAP:
        return "SHARED_PMAP";
    case VM_MEMORY_DYLIB:
        return "DYLIB";
    case VM_MEMORY_OBJC_DISPATCHERS:
        return "OBJC_DISPATCHERS";
    case VM_MEMORY_UNSHARED_PMAP:
        return "UNSHARED_PMAP";
    case VM_MEMORY_APPKIT:
        return "APPKIT";
    case VM_MEMORY_FOUNDATION:
        return "FOUNDATION";
    case VM_MEMORY_COREGRAPHICS:
        return "COREGRAPHICS";
    case VM_MEMORY_CORESERVICES:
        return "CORESERVICES";
    case VM_MEMORY_JAVA:
        return "JAVA";
    case VM_MEMORY_COREDATA:
        return "COREDATA";
    case VM_MEMORY_COREDATA_OBJECTIDS:
        return "COREDATA_OBJECTIDS";
    case VM_MEMORY_ATS:
        return "ATS";
    case VM_MEMORY_LAYERKIT:
        return "LAYERKIT";
    case VM_MEMORY_CGIMAGE:
        return "CGIMAGE";
    case VM_MEMORY_TCMALLOC:
        return "TCMALLOC";
    case VM_MEMORY_COREGRAPHICS_DATA:
        return "COREGRAPHICS_DATA";
    case VM_MEMORY_COREGRAPHICS_SHARED:
        return "COREGRAPHICS_SHARED";
    case VM_MEMORY_COREGRAPHICS_FRAMEBUFFERS:
        return "COREGRAPHICS_FRAMEBUFFERS";
    case VM_MEMORY_COREGRAPHICS_BACKINGSTORES:
        return "COREGRAPHICS_BACKINGSTORES";
    case VM_MEMORY_COREGRAPHICS_XALLOC:
        return "COREGRAPHICS_XALLOC";
    case VM_MEMORY_DYLD:
        return "DYLD";
    case VM_MEMORY_DYLD_MALLOC:
        return "DYLD_MALLOC";
    case VM_MEMORY_SQLITE:
        return "SQLITE";
    case VM_MEMORY_JAVASCRIPT_CORE:
        return "JAVASCRIPT_CORE";
    case VM_MEMORY_JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR:
        return "JAVASCRIPT_JIT_EXECUTABLE_ALLOCATOR";
    case VM_MEMORY_JAVASCRIPT_JIT_REGISTER_FILE:
        return "JAVASCRIPT_JIT_REGISTER_FILE";
    case VM_MEMORY_GLSL:
        return "GLSL";
    case VM_MEMORY_OPENCL:
        return "OPENCL";
    case VM_MEMORY_COREIMAGE:
        return "COREIMAGE";
    case VM_MEMORY_WEBCORE_PURGEABLE_BUFFERS:
        return "WEBCORE_PURGEABLE_BUFFERS";
    case VM_MEMORY_IMAGEIO:
        return "IMAGEIO";
    case VM_MEMORY_COREPROFILE:
        return "COREPROFILE";
    case VM_MEMORY_ASSETSD:
        return "ASSETSD";
    case VM_MEMORY_OS_ALLOC_ONCE:
        return "OS_ALLOC_ONCE";
    case VM_MEMORY_LIBDISPATCH:
        return "LIBDISPATCH";
    case VM_MEMORY_ACCELERATE:
        return "ACCELERATE";
    case VM_MEMORY_COREUI:
        return "COREUI";
    case VM_MEMORY_COREUIFILE:
        return "COREUIFILE";
    case VM_MEMORY_GENEALOGY:
        return "GENEALOGY";
    case VM_MEMORY_RAWCAMERA:
        return "RAWCAMERA";
    case VM_MEMORY_CORPSEINFO:
        return "CORPSEINFO";
    case VM_MEMORY_ASL:
        return "ASL";
    case VM_MEMORY_SWIFT_RUNTIME:
        return "SWIFT_RUNTIME";
    case VM_MEMORY_SWIFT_METADATA:
        return "SWIFT_METADATA";
    case VM_MEMORY_DHMM:
        return "DHMM";
    case VM_MEMORY_SCENEKIT:
        return "SCENEKIT";
    case VM_MEMORY_SKYWALK:
        return "SKYWALK";
    case VM_MEMORY_IOSURFACE:
        return "IOSURFACE";
    case VM_MEMORY_LIBNETWORK:
        return "LIBNETWORK";
    case VM_MEMORY_AUDIO:
        return "AUDIO";
    case VM_MEMORY_VIDEOBITSTREAM:
        return "VIDEOBITSTREAM";
    case VM_MEMORY_CM_XPC:
        return "CM_XPC";
    case VM_MEMORY_CM_RPC:
        return "CM_RPC";
    case VM_MEMORY_CM_MEMORYPOOL:
        return "CM_MEMORYPOOL";
    case VM_MEMORY_CM_READCACHE:
        return "CM_READCACHE";
    case VM_MEMORY_CM_CRABS:
        return "CM_CRABS";
    case VM_MEMORY_QUICKLOOK_THUMBNAILS:
        return "QUICKLOOK_THUMBNAILS";
    case VM_MEMORY_ACCOUNTS:
        return "ACCOUNTS";
    case VM_MEMORY_SANITIZER:
        return "SANITIZER";
    case VM_MEMORY_IOACCELERATOR:
        return "IOACCELERATOR";
    case VM_MEMORY_CM_REGWARP:
        return "CM_REGWARP";
    case VM_MEMORY_EAR_DECODER:
        return "EAR_DECODER";
    case VM_MEMORY_COREUI_CACHED_IMAGE_DATA:
        return "COREUI_CACHED_IMAGE_DATA";
    default:
        return "OTHER";
    }
}

ErrorOr<MemoryInfo> get_memory_info()
{
    MemoryInfo info;

    mach_task_basic_info_data_t basic_info {};
    mach_msg_type_number_t count = MACH_TASK_BASIC_INFO_COUNT;
    auto res = task_info(mach_task_self(), MACH_TASK_BASIC_INFO, reinterpret_cast<task_info_t>(&basic_info), &count);
    if (res != KERN_SUCCESS)
        return Core::mach_error_to_error(res);

    info.resident_bytes = basic_info.resident_size;

    task_vm_info_data_t vm_info {};
    count = TASK_VM_INFO_COUNT;
    res = task_info(mach_task_self(), TASK_VM_INFO, reinterpret_cast<task_info_t>(&vm_info), &count);
    if (res != KERN_SUCCESS)
        return Core::mach_error_to_error(res);

    info.phys_footprint = vm_info.phys_footprint;
    // NB: phys_footprint_peak may not be available on all macOS versions
    // Use phys_footprint as a fallback
    info.phys_footprint_peak = vm_info.phys_footprint;

    // Enumerate memory regions
    mach_vm_address_t address = 0;
    mach_vm_size_t size = 0;

    while (true) {
        vm_region_extended_info_data_t extended_info {};
        count = VM_REGION_EXTENDED_INFO_COUNT;
        mach_port_t object_name = MACH_PORT_NULL;

        res = mach_vm_region(mach_task_self(), &address, &size, VM_REGION_EXTENDED_INFO,
            reinterpret_cast<vm_region_info_t>(&extended_info), &count, &object_name);

        if (res != KERN_SUCCESS)
            break;

        MemoryRegion region;
        region.size = size;
        region.resident = static_cast<u64>(extended_info.pages_resident) * vm_page_size;
        region.dirty = static_cast<u64>(extended_info.pages_dirtied) * vm_page_size;
        region.name = vm_tag_to_name(extended_info.user_tag);

        info.regions.append(move(region));

        address += size;
    }

    return info;
}

}
