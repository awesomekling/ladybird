/*
 * Copyright (c) 2020-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2023-2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Atomic.h>
#include <AK/Badge.h>
#include <AK/Debug.h>
#include <AK/Function.h>
#include <AK/HashTable.h>
#include <AK/JsonArray.h>
#include <AK/JsonObject.h>
#include <AK/Platform.h>
#include <AK/StackInfo.h>
#include <AK/TemporaryChange.h>
#include <LibCore/ElapsedTimer.h>
#include <LibGC/CellAllocator.h>
#include <LibGC/Heap.h>
#include <LibGC/HeapBlock.h>
#include <LibGC/NanBoxedValue.h>
#include <LibGC/Root.h>
#include <LibGC/Weak.h>
#include <LibGC/WeakInlines.h>
#include <LibThreading/ConditionVariable.h>
#include <LibThreading/Mutex.h>
#include <LibThreading/Thread.h>
#include <setjmp.h>

#ifdef HAS_ADDRESS_SANITIZER
#    include <sanitizer/asan_interface.h>
#endif

namespace GC {

static Heap* s_the;

Heap& Heap::the()
{
    return *s_the;
}

Heap::Heap(AK::Function<void(HashMap<Cell*, GC::HeapRoot>&)> gather_embedder_roots)
    : m_gather_embedder_roots(move(gather_embedder_roots))
{
    s_the = this;
    static_assert(HeapBlock::min_possible_cell_size <= 32, "Heap Cell tracking uses too much data!");
    m_size_based_cell_allocators.append(make<CellAllocator>(64));
    m_size_based_cell_allocators.append(make<CellAllocator>(96));
    m_size_based_cell_allocators.append(make<CellAllocator>(128));
    m_size_based_cell_allocators.append(make<CellAllocator>(256));
    m_size_based_cell_allocators.append(make<CellAllocator>(512));
    m_size_based_cell_allocators.append(make<CellAllocator>(1024));
    m_size_based_cell_allocators.append(make<CellAllocator>(3072));
}

Heap::~Heap()
{
    collect_garbage(CollectionType::CollectEverything);
}

void Heap::will_allocate(size_t size)
{
    if (should_collect_on_every_allocation()) {
        m_allocated_bytes_since_last_gc = 0;
        collect_garbage();
    } else if (m_allocated_bytes_since_last_gc + size > m_gc_bytes_threshold) {
        m_allocated_bytes_since_last_gc = 0;
        collect_garbage();
    }

    m_allocated_bytes_since_last_gc += size;
}

static void add_possible_value(HashMap<FlatPtr, HeapRoot>& possible_pointers, FlatPtr data, HeapRoot origin, FlatPtr min_block_address, FlatPtr max_block_address)
{
    if constexpr (sizeof(FlatPtr*) == sizeof(NanBoxedValue)) {
        // Because NanBoxedValue stores pointers in non-canonical form we have to check if the top bytes
        // match any pointer-backed tag, in that case we have to extract the pointer to its
        // canonical form and add that as a possible pointer.
        FlatPtr possible_pointer;
        if ((data & SHIFTED_IS_CELL_PATTERN) == SHIFTED_IS_CELL_PATTERN)
            possible_pointer = NanBoxedValue::extract_pointer_bits(data);
        else
            possible_pointer = data;
        if (possible_pointer < min_block_address || possible_pointer > max_block_address)
            return;
        possible_pointers.set(possible_pointer, move(origin));
    } else {
        static_assert((sizeof(NanBoxedValue) % sizeof(FlatPtr*)) == 0);
        if (data < min_block_address || data > max_block_address)
            return;
        // In the 32-bit case we will look at the top and bottom part of NanBoxedValue separately we just
        // add both the upper and lower bytes as possible pointers.
        possible_pointers.set(data, move(origin));
    }
}

void Heap::find_min_and_max_block_addresses(FlatPtr& min_address, FlatPtr& max_address)
{
    min_address = explode_byte(0xff);
    max_address = 0;
    for (auto& allocator : m_all_cell_allocators) {
        min_address = min(min_address, allocator.min_block_address());
        max_address = max(max_address, allocator.max_block_address() + HeapBlock::BLOCK_SIZE);
    }
}

template<typename Callback>
static void for_each_cell_among_possible_pointers(HashTable<HeapBlock*> const& all_live_heap_blocks, HashMap<FlatPtr, HeapRoot>& possible_pointers, Callback callback)
{
    for (auto possible_pointer : possible_pointers.keys()) {
        if (!possible_pointer)
            continue;
        auto* possible_heap_block = HeapBlock::from_cell(reinterpret_cast<Cell const*>(possible_pointer));
        if (!all_live_heap_blocks.contains(possible_heap_block))
            continue;
        if (auto* cell = possible_heap_block->cell_from_possible_pointer(possible_pointer)) {
            callback(cell, possible_pointer);
        }
    }
}

class GraphConstructorVisitor final : public Cell::Visitor {
public:
    explicit GraphConstructorVisitor(Heap& heap, HashMap<Cell*, HeapRoot> const& roots)
        : m_heap(heap)
    {
        m_heap.find_min_and_max_block_addresses(m_min_block_address, m_max_block_address);
        m_heap.for_each_block([&](auto& block) {
            m_all_live_heap_blocks.set(&block);
            return IterationDecision::Continue;
        });
        m_work_queue.ensure_capacity(roots.size());

        for (auto& [root, root_origin] : roots) {
            auto& graph_node = m_graph.ensure(bit_cast<FlatPtr>(root));
            graph_node.class_name = root->class_name();
            graph_node.root_origin = root_origin;

            m_work_queue.append(*root);
        }
    }

    virtual void visit_impl(Cell& cell) override
    {
        if (m_node_being_visited)
            m_node_being_visited->edges.set(reinterpret_cast<FlatPtr>(&cell));

        if (m_graph.get(reinterpret_cast<FlatPtr>(&cell)).has_value())
            return;

        m_work_queue.append(cell);
    }

    virtual void visit_impl(ReadonlySpan<NanBoxedValue> values) override
    {
        for (auto const& value : values)
            visit(value);
    }

    virtual void visit_possible_values(ReadonlyBytes bytes) override
    {
        HashMap<FlatPtr, HeapRoot> possible_pointers;

        auto* raw_pointer_sized_values = reinterpret_cast<FlatPtr const*>(bytes.data());
        for (size_t i = 0; i < (bytes.size() / sizeof(FlatPtr)); ++i)
            add_possible_value(possible_pointers, raw_pointer_sized_values[i], HeapRoot { .type = HeapRoot::Type::HeapFunctionCapturedPointer }, m_min_block_address, m_max_block_address);

        for_each_cell_among_possible_pointers(m_all_live_heap_blocks, possible_pointers, [&](Cell* cell, FlatPtr) {
            if (cell->state() != Cell::State::Live)
                return;

            if (m_node_being_visited)
                m_node_being_visited->edges.set(reinterpret_cast<FlatPtr>(cell));

            if (m_graph.get(reinterpret_cast<FlatPtr>(cell)).has_value())
                return;
            m_work_queue.append(*cell);
        });
    }

    void visit_all_cells()
    {
        while (!m_work_queue.is_empty()) {
            auto cell = m_work_queue.take_last();
            m_node_being_visited = &m_graph.ensure(bit_cast<FlatPtr>(cell.ptr()));
            m_node_being_visited->class_name = cell->class_name();
            cell->visit_edges(*this);
            m_node_being_visited = nullptr;
        }
    }

    AK::JsonObject dump()
    {
        auto graph = AK::JsonObject();
        for (auto& it : m_graph) {
            AK::JsonArray edges;
            for (auto const& value : it.value.edges) {
                edges.must_append(MUST(String::formatted("{}", value)));
            }

            auto node = AK::JsonObject();
            if (it.value.root_origin.has_value()) {
                auto type = it.value.root_origin->type;
                auto location = it.value.root_origin->location;
                switch (type) {
                case HeapRoot::Type::ConservativeVector:
                    node.set("root"sv, "ConservativeVector"sv);
                    break;
                case HeapRoot::Type::MustSurviveGC:
                    node.set("root"sv, "MustSurviveGC"sv);
                    break;
                case HeapRoot::Type::Root:
                    node.set("root"sv, MUST(String::formatted("Root {} {}:{}", location->function_name(), location->filename(), location->line_number())));
                    break;
                case HeapRoot::Type::RootVector:
                    node.set("root"sv, "RootVector"sv);
                    break;
                case HeapRoot::Type::RegisterPointer:
                    node.set("root"sv, "RegisterPointer"sv);
                    break;
                case HeapRoot::Type::StackPointer:
                    node.set("root"sv, "StackPointer"sv);
                    break;
                case HeapRoot::Type::VM:
                    node.set("root"sv, "VM"sv);
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
            }
            node.set("class_name"sv, it.value.class_name);
            node.set("edges"sv, edges);
            graph.set(ByteString::number(it.key), node);
        }

        return graph;
    }

private:
    struct GraphNode {
        Optional<HeapRoot> root_origin;
        StringView class_name;
        HashTable<FlatPtr> edges {};
    };

    GraphNode* m_node_being_visited { nullptr };
    Vector<Ref<Cell>> m_work_queue;
    HashMap<FlatPtr, GraphNode> m_graph;

    Heap& m_heap;
    HashTable<HeapBlock*> m_all_live_heap_blocks;
    FlatPtr m_min_block_address;
    FlatPtr m_max_block_address;
};

AK::JsonObject Heap::dump_graph()
{
    HashMap<Cell*, HeapRoot> roots;
    HashTable<HeapBlock*> all_live_heap_blocks;
    gather_roots(roots, all_live_heap_blocks);
    GraphConstructorVisitor visitor(*this, roots);
    visitor.visit_all_cells();
    return visitor.dump();
}

void Heap::collect_garbage(CollectionType collection_type, bool print_report)
{
    VERIFY(!m_collecting_garbage);

    {
        TemporaryChange change(m_collecting_garbage, true);

        Core::ElapsedTimer collection_measurement_timer;
        if (print_report)
            collection_measurement_timer.start();

        if (collection_type == CollectionType::CollectGarbage) {
            if (m_gc_deferrals) {
                m_should_gc_when_deferral_ends = true;
                return;
            }
            HashMap<Cell*, HeapRoot> roots;
            HashTable<HeapBlock*> all_live_heap_blocks;
            gather_roots(roots, all_live_heap_blocks);
            mark_live_cells(roots, all_live_heap_blocks);
        }
        finalize_unmarked_cells();
        sweep_weak_blocks();
        sweep_dead_cells(print_report, collection_measurement_timer);

        if (print_report)
            dump_allocators();
    }

    run_post_gc_tasks();
}

void Heap::run_post_gc_tasks()
{
    auto tasks = move(m_post_gc_tasks);
    for (auto& task : tasks)
        task();
}

void Heap::dump_allocators()
{
    size_t total_in_committed_blocks = 0;
    size_t total_waste = 0;
    for (auto& allocator : m_all_cell_allocators) {
        struct BlockStats {
            HeapBlock& block;
            size_t live_cells { 0 };
            size_t dead_cells { 0 };
            size_t total_cells { 0 };
        };
        Vector<BlockStats> blocks;

        size_t total_live_cells = 0;
        size_t total_dead_cells = 0;
        size_t cell_count = (HeapBlock::BLOCK_SIZE - sizeof(HeapBlock)) / allocator.cell_size();

        allocator.for_each_block([&](HeapBlock& heap_block) {
            BlockStats block { heap_block };

            heap_block.for_each_cell([&](Cell* cell) {
                if (cell->state() == Cell::State::Live)
                    ++block.live_cells;
                else if (cell->state() == Cell::State::Dead)
                    ++block.dead_cells;
                else
                    VERIFY_NOT_REACHED();
            });
            total_live_cells += block.live_cells;
            total_dead_cells += block.dead_cells;

            blocks.append({ block });
            return IterationDecision::Continue;
        });

        if (blocks.is_empty())
            continue;

        total_in_committed_blocks += blocks.size() * HeapBlock::BLOCK_SIZE;

        StringBuilder builder;
        if (allocator.class_name().is_null())
            builder.appendff("generic ({}b)", allocator.cell_size());
        else
            builder.appendff("{} ({}b)", allocator.class_name(), allocator.cell_size());

        builder.appendff(" x {}", total_live_cells);

        size_t cost = blocks.size() * HeapBlock::BLOCK_SIZE / KiB;
        size_t reserved = allocator.block_allocator().blocks().size() * HeapBlock::BLOCK_SIZE / KiB;
        builder.appendff(", cost: {} KiB, reserved: {} KiB", cost, reserved);

        size_t total_dead_bytes = ((blocks.size() * cell_count) - total_live_cells) * allocator.cell_size();
        if (total_dead_bytes) {
            builder.appendff(", waste: {} KiB", total_dead_bytes / KiB);
            total_waste += total_dead_bytes;
        }

        dbgln("{}", builder.string_view());

        for (auto& block : blocks) {
            dbgln("  block at {:p}: live {} / dead {} / total {} cells", &block.block, block.live_cells, block.dead_cells, block.block.cell_count());
        }
    }
    dbgln("Total allocated: {} KiB", total_in_committed_blocks / KiB);
    dbgln("Total wasted on fragmentation: {} KiB", total_waste / KiB);
}

void Heap::enqueue_post_gc_task(AK::Function<void()> task)
{
    m_post_gc_tasks.append(move(task));
}

void Heap::gather_roots(HashMap<Cell*, HeapRoot>& roots, HashTable<HeapBlock*>& all_live_heap_blocks)
{
    for_each_block([&](auto& block) {
        all_live_heap_blocks.set(&block);

        if (block.overrides_must_survive_garbage_collection()) {
            block.template for_each_cell_in_state<Cell::State::Live>([&](Cell* cell) {
                if (cell->must_survive_garbage_collection()) {
                    roots.set(cell, HeapRoot { .type = HeapRoot::Type::MustSurviveGC });
                }
            });
        }

        return IterationDecision::Continue;
    });

    m_gather_embedder_roots(roots);
    gather_conservative_roots(roots, all_live_heap_blocks);

    for (auto& root : m_roots)
        roots.set(root.cell(), HeapRoot { .type = HeapRoot::Type::Root, .location = &root.source_location() });

    for (auto& vector : m_root_vectors)
        vector.gather_roots(roots);

    for (auto& hash_map : m_root_hash_maps)
        hash_map.gather_roots(roots);

    if constexpr (HEAP_DEBUG) {
        dbgln("gather_roots:");
        for (auto* root : roots.keys())
            dbgln("  + {}", root);
    }
}

#ifdef HAS_ADDRESS_SANITIZER
NO_SANITIZE_ADDRESS void Heap::gather_asan_fake_stack_roots(HashMap<FlatPtr, HeapRoot>& possible_pointers, FlatPtr addr, FlatPtr min_block_address, FlatPtr max_block_address)
{
    void* begin = nullptr;
    void* end = nullptr;
    void* real_stack = __asan_addr_is_in_fake_stack(__asan_get_current_fake_stack(), reinterpret_cast<void*>(addr), &begin, &end);

    if (real_stack != nullptr) {
        for (auto* real_stack_addr = reinterpret_cast<void const* const*>(begin); real_stack_addr < end; ++real_stack_addr) {
            void const* real_address = *real_stack_addr;
            if (real_address == nullptr)
                continue;
            add_possible_value(possible_pointers, reinterpret_cast<FlatPtr>(real_address), HeapRoot { .type = HeapRoot::Type::StackPointer }, min_block_address, max_block_address);
        }
    }
}
#else
void Heap::gather_asan_fake_stack_roots(HashMap<FlatPtr, HeapRoot>&, FlatPtr, FlatPtr, FlatPtr)
{
}
#endif

NO_SANITIZE_ADDRESS void Heap::gather_conservative_roots(HashMap<Cell*, HeapRoot>& roots, HashTable<HeapBlock*> const& all_live_heap_blocks)
{
    FlatPtr dummy;

    dbgln_if(HEAP_DEBUG, "gather_conservative_roots:");

    jmp_buf buf;
    setjmp(buf);

    HashMap<FlatPtr, HeapRoot> possible_pointers;

    auto* raw_jmp_buf = reinterpret_cast<FlatPtr const*>(buf);

    FlatPtr min_block_address, max_block_address;
    find_min_and_max_block_addresses(min_block_address, max_block_address);

    for (size_t i = 0; i < ((size_t)sizeof(buf)) / sizeof(FlatPtr); ++i)
        add_possible_value(possible_pointers, raw_jmp_buf[i], HeapRoot { .type = HeapRoot::Type::RegisterPointer }, min_block_address, max_block_address);

    auto stack_reference = bit_cast<FlatPtr>(&dummy);

    for (FlatPtr stack_address = stack_reference; stack_address < m_stack_info.top(); stack_address += sizeof(FlatPtr)) {
        auto data = *reinterpret_cast<FlatPtr*>(stack_address);
        add_possible_value(possible_pointers, data, HeapRoot { .type = HeapRoot::Type::StackPointer }, min_block_address, max_block_address);
        gather_asan_fake_stack_roots(possible_pointers, data, min_block_address, max_block_address);
    }

    for (auto& vector : m_conservative_vectors) {
        for (auto possible_value : vector.possible_values()) {
            add_possible_value(possible_pointers, possible_value, HeapRoot { .type = HeapRoot::Type::ConservativeVector }, min_block_address, max_block_address);
        }
    }

    for_each_cell_among_possible_pointers(all_live_heap_blocks, possible_pointers, [&](Cell* cell, FlatPtr possible_pointer) {
        if (cell->state() == Cell::State::Live) {
            dbgln_if(HEAP_DEBUG, "  ?-> {}", (void const*)cell);
            roots.set(cell, *possible_pointers.get(possible_pointer));
        } else {
            dbgln_if(HEAP_DEBUG, "  #-> {}", (void const*)cell);
        }
    });
}

// Parallel marking uses a segment-based work distribution scheme:
//
// - A "segment" is a fixed-size batch of cell pointers waiting to be marked.
// - The main thread discovers cells during root scanning and edge traversal,
//   filling segments and periodically donating full segments to a shared pool.
// - Worker threads pick up segments from the shared pool, visit edges of
//   the (already marked) cells, and mark any newly discovered cells.
// - Each thread maintains local segments to minimize contention,
//   only accessing the shared pool when donating excess work or when empty.
// - The shared work queue is lock-free (atomic stack), while segment
//   allocation uses a spinlock to avoid expensive pthread mutex calls.
// - Workers persist between GC cycles to avoid thread creation overhead.
//
// Performance characteristics:
// - For large object graphs (5M+ objects), provides ~30% speedup over
//   single-threaded marking.
// - For smaller heaps, overhead roughly equals parallel benefit (break-even).
// - Scaling beyond 2 threads shows diminishing returns due to work
//   distribution being inherently unbalanced (main thread finds most roots).

static constexpr size_t SEGMENT_CAPACITY = 256;
static constexpr size_t NUM_MARKING_THREADS = 2;
static constexpr size_t CELLS_BETWEEN_DONATION_CHECK = 256;
static constexpr size_t WORKER_LOCAL_SEGMENTS_THRESHOLD = 4;

// A fixed-size batch of cell pointers. Using fixed arrays amortizes allocation
// overhead and allows O(1) segment donation between threads.
struct MarkSegment {
    Cell* cells[SEGMENT_CAPACITY];
    size_t count { 0 };
    MarkSegment* next { nullptr };

    bool is_full() const { return count >= SEGMENT_CAPACITY; }
    bool is_empty() const { return count == 0; }

    void push(Cell* cell)
    {
        VERIFY(count < SEGMENT_CAPACITY);
        cells[count++] = cell;
    }

    Cell* pop()
    {
        VERIFY(count > 0);
        return cells[--count];
    }

    void clear()
    {
        count = 0;
        next = nullptr;
    }
};

// Pool for shared work segments between threads.
// The shared work queue is lock-free, segment allocation uses a spinlock.
class SegmentPool {
public:
    SegmentPool()
        : m_cond(m_cond_mutex)
    {
        // Pre-allocate segments
        for (size_t i = 0; i < 128; ++i)
            m_free_segments.append(new MarkSegment());
    }

    ~SegmentPool()
    {
        for (auto* seg : m_free_segments)
            delete seg;
        while (auto* seg = try_take_segment())
            delete seg;
    }

    // Lock-free push to shared work queue
    void donate_segment(MarkSegment* segment)
    {
        MarkSegment* old_head = m_shared_head.load(AK::MemoryOrder::memory_order_acquire);
        do {
            segment->next = old_head;
        } while (!m_shared_head.compare_exchange_strong(old_head, segment, AK::MemoryOrder::memory_order_acq_rel));
    }

    // Lock-free pop from shared work queue
    MarkSegment* try_take_segment()
    {
        MarkSegment* old_head = m_shared_head.load(AK::MemoryOrder::memory_order_acquire);
        while (old_head) {
            // Read next before CAS - if CAS fails, old_head is updated to current head
            MarkSegment* next = old_head->next;
            if (m_shared_head.compare_exchange_strong(old_head, next, AK::MemoryOrder::memory_order_acq_rel)) {
                old_head->next = nullptr;
                return old_head;
            }
        }
        return nullptr;
    }

    // Spinlock-protected segment allocation
    MarkSegment* allocate_segment()
    {
        while (m_alloc_lock.exchange(true, AK::MemoryOrder::memory_order_acquire)) { }
        MarkSegment* seg = nullptr;
        if (!m_free_segments.is_empty())
            seg = m_free_segments.take_last();
        m_alloc_lock.store(false, AK::MemoryOrder::memory_order_release);
        return seg ? seg : new MarkSegment();
    }

    // Spinlock-protected segment freeing
    void free_segment(MarkSegment* segment)
    {
        segment->clear();
        while (m_alloc_lock.exchange(true, AK::MemoryOrder::memory_order_acquire)) { }
        m_free_segments.append(segment);
        m_alloc_lock.store(false, AK::MemoryOrder::memory_order_release);
    }

    void clear_shared()
    {
        while (auto* seg = try_take_segment()) {
            seg->clear();
            while (m_alloc_lock.exchange(true, AK::MemoryOrder::memory_order_acquire)) { }
            m_free_segments.append(seg);
            m_alloc_lock.store(false, AK::MemoryOrder::memory_order_release);
        }
    }

    void wait(AK::Atomic<bool> const& marking_active, AK::Atomic<bool> const& shutdown, size_t& last_seen_cycle, AK::Atomic<size_t> const& current_cycle)
    {
        Threading::MutexLocker locker(m_cond_mutex);
        m_cond.wait_while([&] {
            return !marking_active.load(AK::MemoryOrder::memory_order_acquire)
                && !shutdown.load(AK::MemoryOrder::memory_order_acquire)
                && last_seen_cycle == current_cycle.load(AK::MemoryOrder::memory_order_acquire);
        });
        last_seen_cycle = current_cycle.load(AK::MemoryOrder::memory_order_acquire);
    }

    void notify_all()
    {
        Threading::MutexLocker locker(m_cond_mutex);
        m_cond.broadcast();
    }

private:
    // Lock-free shared work queue
    AK::Atomic<MarkSegment*> m_shared_head { nullptr };

    // Spinlock-protected free segment pool
    AK::Atomic<bool> m_alloc_lock { false };
    Vector<MarkSegment*> m_free_segments;

    Threading::Mutex m_cond_mutex;
    Threading::ConditionVariable m_cond;
};

// Singleton thread pool that manages worker threads for parallel marking.
// Workers are created once and persist across GC cycles to avoid thread creation overhead.
class MarkingThreadPool {
public:
    static MarkingThreadPool& the()
    {
        static MarkingThreadPool instance;
        return instance;
    }

    void start_marking(HashTable<HeapBlock*> const& all_live_heap_blocks, FlatPtr min_block_address, FlatPtr max_block_address)
    {
        m_segment_pool.clear_shared();
        m_workers_finished_count.store(0, AK::MemoryOrder::memory_order_release);
        m_all_live_heap_blocks = &all_live_heap_blocks;
        m_min_block_address = min_block_address;
        m_max_block_address = max_block_address;
        m_current_cycle.fetch_add(1, AK::MemoryOrder::memory_order_acq_rel);
        m_marking_active.store(true, AK::MemoryOrder::memory_order_release);
        m_segment_pool.notify_all();
    }

    HashTable<HeapBlock*> const& all_live_heap_blocks() const { return *m_all_live_heap_blocks; }
    FlatPtr min_block_address() const { return m_min_block_address; }
    FlatPtr max_block_address() const { return m_max_block_address; }

    void stop_marking()
    {
        m_marking_active.store(false, AK::MemoryOrder::memory_order_release);

        // Wait for all workers to finish, keep notifying to wake any that are waiting
        while (m_workers_finished_count.load(AK::MemoryOrder::memory_order_acquire) < NUM_MARKING_THREADS) {
            m_segment_pool.notify_all();
        }
    }

    SegmentPool& segment_pool() { return m_segment_pool; }

private:
    MarkingThreadPool()
    {
        for (size_t i = 0; i < NUM_MARKING_THREADS; ++i) {
            auto thread = Threading::Thread::construct([this]() -> intptr_t {
                worker_main();
                return 0;
            });
            thread->start();
            m_workers.append(thread);
        }

        // Wait for all workers to be ready before returning
        while (m_workers_ready_count.load(AK::MemoryOrder::memory_order_acquire) < NUM_MARKING_THREADS) {
            // Spin until all workers have started
        }
    }

    ~MarkingThreadPool()
    {
        m_shutdown.store(true, AK::MemoryOrder::memory_order_release);
        m_segment_pool.notify_all();
        for (auto& thread : m_workers)
            (void)thread->join();
    }

    // Visitor used by worker threads. Marks cells, traverses edges, and manages
    // local work segments to minimize contention with the shared pool.
    class WorkerVisitor final : public Cell::Visitor {
    public:
        WorkerVisitor(MarkingThreadPool& thread_pool)
            : m_thread_pool(thread_pool)
            , m_segment_pool(thread_pool.segment_pool())
        {
        }

        ~WorkerVisitor()
        {
            flush();
        }

        virtual void visit_impl(Cell& cell) override
        {
            if (cell.is_marked())
                return;
            cell.set_marked(true);

            if (!m_current_segment)
                m_current_segment = m_segment_pool.allocate_segment();

            m_current_segment->push(&cell);

            if (m_current_segment->is_full()) {
                m_local_segments.append(m_current_segment);
                m_current_segment = nullptr;
            }
        }

        virtual void visit_impl(ReadonlySpan<NanBoxedValue> values) override
        {
            for (auto value : values) {
                if (!value.is_cell())
                    continue;
                auto& cell = value.as_cell();
                visit_impl(cell);
            }
        }

        virtual void visit_possible_values(ReadonlyBytes bytes) override
        {
            HashMap<FlatPtr, HeapRoot> possible_pointers;
            auto* raw_pointer_sized_values = reinterpret_cast<FlatPtr const*>(bytes.data());
            for (size_t i = 0; i < (bytes.size() / sizeof(FlatPtr)); ++i)
                add_possible_value(possible_pointers, raw_pointer_sized_values[i], HeapRoot { .type = HeapRoot::Type::HeapFunctionCapturedPointer }, m_thread_pool.min_block_address(), m_thread_pool.max_block_address());

            for_each_cell_among_possible_pointers(m_thread_pool.all_live_heap_blocks(), possible_pointers, [&](Cell* cell, FlatPtr) {
                if (cell->is_marked())
                    return;
                if (cell->state() != Cell::State::Live)
                    return;
                cell->set_marked(true);

                if (!m_current_segment)
                    m_current_segment = m_segment_pool.allocate_segment();
                m_current_segment->push(cell);
                if (m_current_segment->is_full()) {
                    m_local_segments.append(m_current_segment);
                    m_current_segment = nullptr;
                }
            });
        }

        // Process local work and return true if we did work, false if empty
        bool drain_local_work()
        {
            if (!m_current_segment && m_local_segments.is_empty())
                return false;

            while (!m_local_segments.is_empty() || (m_current_segment && !m_current_segment->is_empty())) {
                // Donate excess segments to help other workers
                while (m_local_segments.size() > WORKER_LOCAL_SEGMENTS_THRESHOLD) {
                    m_segment_pool.donate_segment(m_local_segments.take_first());
                }

                // Get a segment to work on - either current or from local queue
                if (!m_current_segment || m_current_segment->is_empty()) {
                    if (!m_local_segments.is_empty()) {
                        if (m_current_segment)
                            m_segment_pool.free_segment(m_current_segment);
                        m_current_segment = m_local_segments.take_last();
                    } else {
                        break;
                    }
                }

                Cell* cell = m_current_segment->pop();
                cell->visit_edges(*this);
            }
            return true;
        }

        void flush()
        {
            // Donate any remaining work back to the pool
            for (auto* seg : m_local_segments) {
                if (!seg->is_empty())
                    m_segment_pool.donate_segment(seg);
                else
                    m_segment_pool.free_segment(seg);
            }
            m_local_segments.clear();

            if (m_current_segment) {
                if (!m_current_segment->is_empty())
                    m_segment_pool.donate_segment(m_current_segment);
                else
                    m_segment_pool.free_segment(m_current_segment);
                m_current_segment = nullptr;
            }
        }

    private:
        MarkingThreadPool& m_thread_pool;
        SegmentPool& m_segment_pool;
        MarkSegment* m_current_segment { nullptr };
        Vector<MarkSegment*> m_local_segments;
    };

    void worker_main()
    {
        WorkerVisitor visitor(*this);
        size_t last_seen_cycle = 0;

        // Signal that this worker is ready
        m_workers_ready_count.fetch_add(1, AK::MemoryOrder::memory_order_release);

        while (!m_shutdown.load(AK::MemoryOrder::memory_order_acquire)) {
            // Wait for a new GC cycle to start
            m_segment_pool.wait(m_marking_active, m_shutdown, last_seen_cycle, m_current_cycle);

            if (m_shutdown.load(AK::MemoryOrder::memory_order_acquire))
                break;

            // If marking already finished before we woke up, signal done
            if (!m_marking_active.load(AK::MemoryOrder::memory_order_acquire)) {
                m_workers_finished_count.fetch_add(1, AK::MemoryOrder::memory_order_release);
                continue;
            }

            // Active marking phase - poll for work
            while (m_marking_active.load(AK::MemoryOrder::memory_order_acquire)) {
                MarkSegment* segment = m_segment_pool.try_take_segment();
                if (!segment)
                    continue;

                // Process segment cells - they're already marked, just need edges visited
                while (!segment->is_empty()) {
                    Cell* cell = segment->pop();
                    cell->visit_edges(visitor);
                }
                m_segment_pool.free_segment(segment);

                // Process all discovered work locally
                visitor.drain_local_work();
            }

            // Flush any remaining work and signal finished
            visitor.flush();
            m_workers_finished_count.fetch_add(1, AK::MemoryOrder::memory_order_release);
        }
    }

    SegmentPool m_segment_pool;
    Vector<NonnullRefPtr<Threading::Thread>> m_workers;
    AK::Atomic<bool> m_shutdown { false };
    AK::Atomic<bool> m_marking_active { false };
    AK::Atomic<size_t> m_workers_ready_count { 0 };
    AK::Atomic<size_t> m_workers_finished_count { 0 };
    AK::Atomic<size_t> m_current_cycle { 0 };

    // Per-cycle data for conservative scanning (set by start_marking)
    HashTable<HeapBlock*> const* m_all_live_heap_blocks { nullptr };
    FlatPtr m_min_block_address { 0 };
    FlatPtr m_max_block_address { 0 };
};

// Visitor used by the main thread during parallel marking. Handles root scanning,
// conservative pointer analysis, and periodically donates work to the shared pool.
class ParallelMarkingVisitor final : public Cell::Visitor {
public:
    ParallelMarkingVisitor(Heap& heap, HashTable<HeapBlock*> const& all_live_heap_blocks, SegmentPool& pool)
        : m_heap(heap)
        , m_all_live_heap_blocks(all_live_heap_blocks)
        , m_segment_pool(pool)
    {
        m_heap.find_min_and_max_block_addresses(m_min_block_address, m_max_block_address);
        m_current_segment = m_segment_pool.allocate_segment();
    }

    ~ParallelMarkingVisitor()
    {
        if (m_current_segment)
            m_segment_pool.free_segment(m_current_segment);
    }

    void add_root(Cell* cell)
    {
        if (cell->is_marked())
            return;
        cell->set_marked(true);
        push_cell(cell);
    }

    virtual void visit_impl(Cell& cell) override
    {
        if (cell.is_marked())
            return;
        cell.set_marked(true);
        push_cell(&cell);
    }

    virtual void visit_impl(ReadonlySpan<NanBoxedValue> values) override
    {
        for (auto value : values) {
            if (!value.is_cell())
                continue;
            auto& cell = value.as_cell();
            if (cell.is_marked())
                continue;
            cell.set_marked(true);
            push_cell(&cell);
        }
    }

    virtual void visit_possible_values(ReadonlyBytes bytes) override
    {
        HashMap<FlatPtr, HeapRoot> possible_pointers;
        auto* raw_pointer_sized_values = reinterpret_cast<FlatPtr const*>(bytes.data());
        for (size_t i = 0; i < (bytes.size() / sizeof(FlatPtr)); ++i)
            add_possible_value(possible_pointers, raw_pointer_sized_values[i], HeapRoot { .type = HeapRoot::Type::HeapFunctionCapturedPointer }, m_min_block_address, m_max_block_address);

        for_each_cell_among_possible_pointers(m_all_live_heap_blocks, possible_pointers, [&](Cell* cell, FlatPtr) {
            if (cell->is_marked())
                return;
            if (cell->state() != Cell::State::Live)
                return;
            cell->set_marked(true);
            push_cell(cell);
        });
    }

    void drain_local_work()
    {
        size_t cells_since_donation_check = 0;

        while (!m_current_segment->is_empty() || !m_local_segments.is_empty()) {
            if (m_current_segment->is_empty() && !m_local_segments.is_empty()) {
                m_segment_pool.free_segment(m_current_segment);
                m_current_segment = m_local_segments.take_last();
            }

            if (m_current_segment->is_empty())
                break;

            Cell* cell = m_current_segment->pop();
            cell->visit_edges(*this);

            if (++cells_since_donation_check >= CELLS_BETWEEN_DONATION_CHECK) {
                cells_since_donation_check = 0;
                maybe_donate_work();
            }
        }
    }

private:
    void push_cell(Cell* cell)
    {
        if (m_current_segment->is_full()) {
            m_local_segments.append(m_current_segment);
            m_current_segment = m_segment_pool.allocate_segment();
        }
        m_current_segment->push(cell);
    }

    void maybe_donate_work()
    {
        if (!m_local_segments.is_empty()) {
            m_segment_pool.donate_segment(m_local_segments.take_first());
        }
    }

    Heap& m_heap;
    HashTable<HeapBlock*> const& m_all_live_heap_blocks;
    SegmentPool& m_segment_pool;
    FlatPtr m_min_block_address;
    FlatPtr m_max_block_address;

    MarkSegment* m_current_segment { nullptr };
    Vector<MarkSegment*> m_local_segments;
};

class MarkingVisitor final : public Cell::Visitor {
public:
    explicit MarkingVisitor(Heap& heap, HashMap<Cell*, HeapRoot> const& roots, HashTable<HeapBlock*> const& all_live_heap_blocks)
        : m_heap(heap)
        , m_all_live_heap_blocks(all_live_heap_blocks)
    {
        m_heap.find_min_and_max_block_addresses(m_min_block_address, m_max_block_address);
        for (auto* root : roots.keys()) {
            visit(root);
        }
    }

    virtual void visit_impl(Cell& cell) override
    {
        if (cell.is_marked())
            return;
        dbgln_if(HEAP_DEBUG, "  ! {}", &cell);

        cell.set_marked(true);
        m_work_queue.append(cell);
    }

    virtual void visit_impl(ReadonlySpan<NanBoxedValue> values) override
    {
        m_work_queue.ensure_capacity(m_work_queue.size() + values.size());

        for (auto value : values) {
            if (!value.is_cell())
                continue;
            auto& cell = value.as_cell();
            if (cell.is_marked())
                continue;
            dbgln_if(HEAP_DEBUG, "  ! {}", &cell);

            cell.set_marked(true);
            m_work_queue.unchecked_append(cell);
        }
    }

    virtual void visit_possible_values(ReadonlyBytes bytes) override
    {
        HashMap<FlatPtr, HeapRoot> possible_pointers;

        auto* raw_pointer_sized_values = reinterpret_cast<FlatPtr const*>(bytes.data());
        for (size_t i = 0; i < (bytes.size() / sizeof(FlatPtr)); ++i)
            add_possible_value(possible_pointers, raw_pointer_sized_values[i], HeapRoot { .type = HeapRoot::Type::HeapFunctionCapturedPointer }, m_min_block_address, m_max_block_address);

        for_each_cell_among_possible_pointers(m_all_live_heap_blocks, possible_pointers, [&](Cell* cell, FlatPtr) {
            if (cell->is_marked())
                return;
            if (cell->state() != Cell::State::Live)
                return;
            cell->set_marked(true);
            m_work_queue.append(*cell);
        });
    }

    void mark_all_live_cells()
    {
        while (!m_work_queue.is_empty()) {
            m_work_queue.take_last()->visit_edges(*this);
        }
    }

private:
    Heap& m_heap;
    Vector<Ref<Cell>> m_work_queue;
    HashTable<HeapBlock*> const& m_all_live_heap_blocks;
    FlatPtr m_min_block_address;
    FlatPtr m_max_block_address;
};

void Heap::mark_live_cells(HashMap<Cell*, HeapRoot> const& roots, HashTable<HeapBlock*> const& all_live_heap_blocks)
{
    dbgln_if(HEAP_DEBUG, "mark_live_cells:");

    if constexpr (NUM_MARKING_THREADS > 0) {
        // Use persistent thread pool for parallel marking
        auto& pool = MarkingThreadPool::the();
        auto& segment_pool = pool.segment_pool();

        // Compute block address range for conservative scanning
        FlatPtr min_block_address = 0, max_block_address = 0;
        find_min_and_max_block_addresses(min_block_address, max_block_address);

        // Signal workers to start
        pool.start_marking(all_live_heap_blocks, min_block_address, max_block_address);

        // Create main thread visitor and add roots
        ParallelMarkingVisitor main_visitor(*this, all_live_heap_blocks, segment_pool);
        for (auto* root : roots.keys()) {
            main_visitor.add_root(root);
        }

        // Main thread processes its work and donates to shared pool
        main_visitor.drain_local_work();

        // Help drain remaining shared work - workers may still be producing more
        // Keep helping until the pool stays empty
        size_t empty_checks = 0;
        while (empty_checks < 3) {
            if (auto* segment = segment_pool.try_take_segment()) {
                empty_checks = 0;
                while (!segment->is_empty()) {
                    Cell* cell = segment->pop();
                    cell->visit_edges(main_visitor); // Cell is already marked
                }
                segment_pool.free_segment(segment);
                main_visitor.drain_local_work();
            } else {
                ++empty_checks;
            }
        }

        // Signal workers to stop and wait for them to finish
        pool.stop_marking();

        // Drain any work that workers flushed back to the pool
        while (auto* segment = segment_pool.try_take_segment()) {
            while (!segment->is_empty()) {
                Cell* cell = segment->pop();
                cell->visit_edges(main_visitor);
            }
            segment_pool.free_segment(segment);
            main_visitor.drain_local_work();
        }
    } else {
        // Single-threaded marking (original implementation)
        MarkingVisitor visitor(*this, roots, all_live_heap_blocks);
        visitor.mark_all_live_cells();
    }

    for (auto& inverse_root : m_uprooted_cells)
        inverse_root->set_marked(false);

    m_uprooted_cells.clear();
}

void Heap::finalize_unmarked_cells()
{
    for_each_block([&](auto& block) {
        if (!block.overrides_finalize())
            return IterationDecision::Continue;
        block.template for_each_cell_in_state<Cell::State::Live>([](Cell* cell) {
            if (!cell->is_marked())
                cell->finalize();
        });
        return IterationDecision::Continue;
    });
}

void Heap::sweep_weak_blocks()
{
    for (auto& weak_block : m_usable_weak_blocks) {
        weak_block.sweep();
    }
    Vector<WeakBlock&> now_usable_weak_blocks;
    for (auto& weak_block : m_full_weak_blocks) {
        weak_block.sweep();
        if (weak_block.can_allocate())
            now_usable_weak_blocks.append(weak_block);
    }
    for (auto& weak_block : now_usable_weak_blocks) {
        m_usable_weak_blocks.append(weak_block);
    }
}

void Heap::sweep_dead_cells(bool print_report, Core::ElapsedTimer const& measurement_timer)
{
    dbgln_if(HEAP_DEBUG, "sweep_dead_cells:");
    Vector<HeapBlock*, 32> empty_blocks;
    Vector<HeapBlock*, 32> full_blocks_that_became_usable;

    size_t collected_cells = 0;
    size_t live_cells = 0;
    size_t collected_cell_bytes = 0;
    size_t live_cell_bytes = 0;

    for_each_block([&](auto& block) {
        bool block_has_live_cells = false;
        bool block_was_full = block.is_full();
        block.template for_each_cell_in_state<Cell::State::Live>([&](Cell* cell) {
            if (!cell->is_marked()) {
                dbgln_if(HEAP_DEBUG, "  ~ {}", cell);
                block.deallocate(cell);
                ++collected_cells;
                collected_cell_bytes += block.cell_size();
            } else {
                cell->set_marked(false);
                block_has_live_cells = true;
                ++live_cells;
                live_cell_bytes += block.cell_size();
            }
        });
        if (!block_has_live_cells)
            empty_blocks.append(&block);
        else if (block_was_full != block.is_full())
            full_blocks_that_became_usable.append(&block);
        return IterationDecision::Continue;
    });

    for (auto& weak_container : m_weak_containers)
        weak_container.remove_dead_cells({});

    for (auto* block : empty_blocks) {
        dbgln_if(HEAP_DEBUG, " - HeapBlock empty @ {}: cell_size={}", block, block->cell_size());
        block->cell_allocator().block_did_become_empty({}, *block);
    }

    for (auto* block : full_blocks_that_became_usable) {
        dbgln_if(HEAP_DEBUG, " - HeapBlock usable again @ {}: cell_size={}", block, block->cell_size());
        block->cell_allocator().block_did_become_usable({}, *block);
    }

    if constexpr (HEAP_DEBUG) {
        for_each_block([&](auto& block) {
            dbgln(" > Live HeapBlock @ {}: cell_size={}", &block, block.cell_size());
            return IterationDecision::Continue;
        });
    }

    m_gc_bytes_threshold = live_cell_bytes > GC_MIN_BYTES_THRESHOLD ? live_cell_bytes : GC_MIN_BYTES_THRESHOLD;

    if (print_report) {
        AK::Duration const time_spent = measurement_timer.elapsed_time();
        size_t live_block_count = 0;
        for_each_block([&](auto&) {
            ++live_block_count;
            return IterationDecision::Continue;
        });

        dbgln("Garbage collection report");
        dbgln("=============================================");
        dbgln("     Time spent: {} ms", time_spent.to_milliseconds());
        dbgln("     Live cells: {} ({} bytes)", live_cells, live_cell_bytes);
        dbgln("Collected cells: {} ({} bytes)", collected_cells, collected_cell_bytes);
        dbgln("    Live blocks: {} ({} bytes)", live_block_count, live_block_count * HeapBlock::BLOCK_SIZE);
        dbgln("   Freed blocks: {} ({} bytes)", empty_blocks.size(), empty_blocks.size() * HeapBlock::BLOCK_SIZE);
        dbgln("=============================================");
    }
}

void Heap::defer_gc()
{
    ++m_gc_deferrals;
}

void Heap::undefer_gc()
{
    VERIFY(m_gc_deferrals > 0);
    --m_gc_deferrals;

    if (!m_gc_deferrals) {
        if (m_should_gc_when_deferral_ends)
            collect_garbage();
        m_should_gc_when_deferral_ends = false;
    }
}

void Heap::uproot_cell(Cell* cell)
{
    m_uprooted_cells.append(cell);
}

WeakImpl* Heap::create_weak_impl(void* ptr)
{
    if (m_usable_weak_blocks.is_empty()) {
        // NOTE: These are leaked on Heap destruction, but that's fine since Heap is tied to process lifetime.
        auto* weak_block = WeakBlock::create();
        m_usable_weak_blocks.append(*weak_block);
    }

    auto* weak_block = m_usable_weak_blocks.first();
    auto* new_weak_impl = weak_block->allocate(static_cast<Cell*>(ptr));
    if (!weak_block->can_allocate()) {
        m_full_weak_blocks.append(*weak_block);
    }

    return new_weak_impl;
}

}
