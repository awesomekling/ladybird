/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, Daniel Bertalan <dani@danielbertalan.dev>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Platform.h>
#include <AK/kmalloc.h>

#if __has_feature(address_sanitizer) || defined(__SANITIZE_ADDRESS__) || defined(HAS_THREAD_SANITIZER)
// LeakSanitizer does not reliably trace references stored in mimalloc-managed
// AK containers, and ThreadSanitizer cannot see memory being freed on one thread
// and reused on another inside mimalloc, so sanitizer builds fall back to the
// system allocator.
#    define AK_USE_SYSTEM_ALLOCATOR_INSTRUMENTED 1
#else
#    include <mimalloc.h>
#    ifdef AK_OS_MACOS
#        include <mach/vm_statistics.h>
#    endif
#    if defined(AK_OS_LINUX) || defined(AK_OS_MACOS)
#        include <sys/mman.h>
#        include <unistd.h>
#    endif
#endif

#ifdef AK_USE_SYSTEM_ALLOCATOR_INSTRUMENTED

#    ifdef AK_OS_MACOS
#        include <malloc/malloc.h>
#    else
#        include <malloc.h>
#    endif

void* ak_kcalloc(size_t count, size_t size)
{
    return calloc(count, size);
}

void* ak_kmalloc(size_t size)
{
    return malloc(size);
}

void* ak_kmalloc(HeapPartition, size_t size)
{
    return ak_kmalloc(size);
}

void* ak_kmalloc_aligned(size_t size, size_t alignment)
{
    void* ptr = nullptr;
    if (posix_memalign(&ptr, alignment, size) != 0)
        return nullptr;
    return ptr;
}

void* ak_kmalloc_aligned(HeapPartition, size_t size, size_t alignment)
{
    return ak_kmalloc_aligned(size, alignment);
}

void* ak_krealloc(void* ptr, size_t size)
{
    return realloc(ptr, size);
}

void* ak_krealloc(HeapPartition, void* ptr, size_t size)
{
    return ak_krealloc(ptr, size);
}

size_t ak_kmalloc_usable_size(void const* ptr)
{
#    ifdef AK_OS_MACOS
    return malloc_size(ptr);
#    else
    return malloc_usable_size(const_cast<void*>(ptr));
#    endif
}

void ak_kfree(void* ptr)
{
    free(ptr);
}

void ak_kmalloc_collect()
{
}

#else

#    if defined(AK_OS_LINUX) || defined(AK_OS_MACOS)
static struct MimallocConfiguration {
    MimallocConfiguration()
    {
#        ifdef AK_OS_MACOS
        // Keep mimalloc allocations separate from IOAccelerator, which uses mimalloc's default tag of 100.
        mi_option_set_default(mi_option_os_tag, VM_MEMORY_APPLICATION_SPECIFIC_1);
#        else
        // mimalloc otherwise purges with MADV_DONTNEED, and every later reuse of a purged page takes a page fault.
        mi_option_set_default(mi_option_purge_decommits, 0);
#        endif
    }
} s_mimalloc_configuration;
#    endif

void* ak_kcalloc(size_t count, size_t size)
{
    return mi_calloc(count, size);
}

void* ak_kmalloc(size_t size)
{
    return mi_malloc(size);
}

static thread_local mi_heap_t* s_buffer_heap = nullptr;
static thread_local mi_heap_t* s_string_heap = nullptr;

static mi_heap_t* heap_for_partition(HeapPartition partition)
{
    switch (partition) {
    case HeapPartition::General:
        return mi_heap_get_default();
    case HeapPartition::ArrayBuffer:
        static mi_heap_t* array_buffer_heap = mi_heap_new();
        return array_buffer_heap;
    case HeapPartition::JSObjectStorage:
        static mi_heap_t* js_object_storage_heap = mi_heap_new();
        return js_object_storage_heap;
    case HeapPartition::Layout:
        static mi_heap_t* layout_heap = mi_heap_new();
        return layout_heap;
    case HeapPartition::Buffer:
        if (!s_buffer_heap)
            s_buffer_heap = mi_heap_new();
        return s_buffer_heap;
    case HeapPartition::String:
        if (!s_string_heap)
            s_string_heap = mi_heap_new();
        return s_string_heap;
    }
    VERIFY_NOT_REACHED();
}

void* ak_kmalloc(HeapPartition partition, size_t size)
{
    return mi_heap_malloc(heap_for_partition(partition), size);
}

void* ak_kmalloc_aligned(size_t size, size_t alignment)
{
    return mi_malloc_aligned(size, alignment);
}

void* ak_kmalloc_aligned(HeapPartition partition, size_t size, size_t alignment)
{
    return mi_heap_malloc_aligned(heap_for_partition(partition), size, alignment);
}

void* ak_krealloc(void* ptr, size_t size)
{
    return mi_realloc(ptr, size);
}

void* ak_krealloc(HeapPartition partition, void* ptr, size_t size)
{
    return mi_heap_realloc(heap_for_partition(partition), ptr, size);
}

size_t ak_kmalloc_usable_size(void const* ptr)
{
    return mi_usable_size(ptr);
}

void ak_kfree(void* ptr)
{
    mi_free(ptr);
}

#    if defined(AK_OS_LINUX) || defined(AK_OS_MACOS)
// A page of large blocks stays committed while any one of them is in use, and the blocks freed around that one keep
// whatever they held resident. The allocator reads nothing of a freed block but the free list link at its start, so the
// rest of it can go back to the kernel.
struct FreedBlockPurge {
    uintptr_t page_size { static_cast<uintptr_t>(sysconf(_SC_PAGESIZE)) };
    uintptr_t block_size { 0 };
    uintptr_t next_block { 0 };
    uintptr_t area_end { 0 };

    void purge_blocks_before(uintptr_t end)
    {
        for (auto block = next_block; block < end; block += block_size) {
            auto start = (block + sizeof(void*) + page_size - 1) & ~(page_size - 1);
            auto stop = (block + block_size) & ~(page_size - 1);
            if (start < stop)
                madvise(reinterpret_cast<void*>(start), stop - start, MADV_FREE);
        }
    }
};

static bool purge_freed_blocks(mi_heap_t const*, mi_heap_area_t const* area, void* block, size_t, void* argument)
{
    auto& purge = *static_cast<FreedBlockPurge*>(argument);
    if (!block) {
        // The blocks of an area are visited in address order, so the ones after the last visited are free.
        purge.purge_blocks_before(purge.area_end);
        purge.block_size = area->full_block_size;
        purge.next_block = reinterpret_cast<uintptr_t>(area->blocks);
        purge.area_end = purge.block_size > purge.page_size ? purge.next_block + area->committed : 0;
        return true;
    }
    if (!purge.area_end)
        return true;
    purge.purge_blocks_before(reinterpret_cast<uintptr_t>(block));
    purge.next_block = reinterpret_cast<uintptr_t>(block) + purge.block_size;
    return true;
}

static void purge_freed_blocks(mi_heap_t* heap)
{
    FreedBlockPurge purge;
    mi_heap_visit_blocks(heap, true, purge_freed_blocks, &purge);
    purge.purge_blocks_before(purge.area_end);
}
#    endif

void ak_kmalloc_collect()
{
    // mi_collect() only visits the calling thread's default heap, so the thread-local partitions have to be collected
    // separately. The remaining partitions are shared between threads and are left to their owners.
    if (s_buffer_heap)
        mi_heap_collect(s_buffer_heap, true);
    if (s_string_heap)
        mi_heap_collect(s_string_heap, true);

    mi_collect(true);

#    if defined(AK_OS_LINUX) || defined(AK_OS_MACOS)
    purge_freed_blocks(mi_heap_get_default());
    if (s_buffer_heap)
        purge_freed_blocks(s_buffer_heap);
    if (s_string_heap)
        purge_freed_blocks(s_string_heap);
#    endif
}

#endif
