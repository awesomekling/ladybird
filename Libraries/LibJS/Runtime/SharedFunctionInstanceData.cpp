/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AtomicRefCounted.h>
#include <AK/Queue.h>
#include <LibJS/Bytecode/Debug.h>
#include <LibJS/Runtime/SharedFunctionInstanceData.h>
#include <LibJS/RustIntegration.h>
#include <LibThreading/ConditionVariable.h>
#include <LibThreading/Mutex.h>
#include <LibThreading/ThreadPool.h>

extern "C" void* rust_create_function_bytecode_input(void*);
extern "C" void rust_free_function_bytecode_input(void*);
extern "C" void rust_start_function_bytecode_input_background_compilation(void*, size_t, bool);
extern "C" void* rust_detach_function_ast_for_background_precompilation(void*);

namespace JS {

GC_DEFINE_ALLOCATOR(SharedFunctionInstanceData);

namespace RustIntegration {

class FunctionBytecodeInput;

static void enqueue_function_bytecode_compilation(FunctionBytecodeInput&, size_t source_len, bool builtin_abstract_operations_enabled, bool urgent);
static void promote_function_bytecode_compilation(FunctionBytecodeInput&, size_t source_len, bool builtin_abstract_operations_enabled);

// Thread-safe Rust-owned input for a lazy function. Background workers may replace the AST payload with precompiled
// bytecode. If the main thread needs the function first, it promotes the existing background job and waits for it.
class FunctionBytecodeInput final
    : public AtomicRefCounted<FunctionBytecodeInput> {
public:
    explicit FunctionBytecodeInput(void* rust_function_ast)
        : m_rust_function_ast(rust_detach_function_ast_for_background_precompilation(rust_function_ast))
    {
    }

    explicit FunctionBytecodeInput(FFI::PrecompiledFunction* precompiled)
        : m_precompiled(precompiled)
        , m_state(State::Precompiled)
    {
    }

    ~FunctionBytecodeInput()
    {
        RustIntegration::free_function_ast(m_rust_function_ast);
        RustIntegration::free_precompiled_function(m_precompiled);
    }

    void set_precompiled(FFI::PrecompiledFunction* precompiled)
    {
        Threading::MutexLocker locker(m_mutex);
        RustIntegration::free_function_ast(m_rust_function_ast);
        RustIntegration::free_precompiled_function(m_precompiled);
        m_rust_function_ast = nullptr;
        m_precompiled = precompiled;
        m_state = State::Precompiled;
        m_condition.broadcast();
    }

    void start_background_compilation(size_t source_len, bool builtin_abstract_operations_enabled)
    {
        Threading::MutexLocker locker(m_mutex);
        if (m_state != State::Uncompiled)
            return;
        m_state = State::Queued;
        enqueue_function_bytecode_compilation(*this, source_len, builtin_abstract_operations_enabled, false);
    }

    FFI::PrecompiledFunction* take_precompiled_or_compile(size_t source_len, bool builtin_abstract_operations_enabled)
    {
        bool should_promote = false;
        {
            Threading::MutexLocker locker(m_mutex);
            if (m_state == State::Precompiled) {
                m_state = State::Empty;
                return exchange(m_precompiled, nullptr);
            }

            if (m_state == State::Uncompiled) {
                m_state = State::Queued;
                enqueue_function_bytecode_compilation(*this, source_len, builtin_abstract_operations_enabled, true);
            } else if (m_state == State::Queued) {
                should_promote = true;
            }
        }

        if (should_promote)
            promote_function_bytecode_compilation(*this, source_len, builtin_abstract_operations_enabled);

        Threading::MutexLocker locker(m_mutex);
        m_condition.wait_while([this] { return m_state == State::Queued || m_state == State::Compiling; });
        VERIFY(m_state == State::Precompiled);
        m_state = State::Empty;
        return exchange(m_precompiled, nullptr);
    }

    void compile_queued(size_t source_len, bool builtin_abstract_operations_enabled)
    {
        void* rust_function_ast = nullptr;
        {
            Threading::MutexLocker locker(m_mutex);
            if (m_state != State::Queued)
                return;
            rust_function_ast = exchange(m_rust_function_ast, nullptr);
            m_state = State::Compiling;
        }

        VERIFY(rust_function_ast);
        auto* precompiled = RustIntegration::precompile_function_off_thread(
            rust_function_ast,
            source_len,
            builtin_abstract_operations_enabled);
        VERIFY(precompiled);

        Threading::MutexLocker locker(m_mutex);
        m_precompiled = precompiled;
        m_state = State::Precompiled;
        m_condition.broadcast();
    }

private:
    enum class State : u8 {
        Empty,
        Uncompiled,
        Queued,
        Compiling,
        Precompiled,
    };

    mutable Threading::Mutex m_mutex;
    Threading::ConditionVariable m_condition { m_mutex };
    void* m_rust_function_ast { nullptr };
    FFI::PrecompiledFunction* m_precompiled { nullptr };
    State m_state { State::Uncompiled };
};

struct FunctionBytecodeCompilationJob {
    RefPtr<FunctionBytecodeInput> input;
    size_t source_len { 0 };
    bool builtin_abstract_operations_enabled { false };
};

static Threading::Mutex& function_bytecode_compilation_queue_mutex()
{
    static Threading::Mutex mutex;
    return mutex;
}

static Queue<FunctionBytecodeCompilationJob>& function_bytecode_compilation_queue()
{
    static Queue<FunctionBytecodeCompilationJob> queue;
    return queue;
}

static Queue<FunctionBytecodeCompilationJob>& urgent_function_bytecode_compilation_queue()
{
    static Queue<FunctionBytecodeCompilationJob> queue;
    return queue;
}

static size_t pending_function_bytecode_compilation_job_count()
{
    return function_bytecode_compilation_queue().size() + urgent_function_bytecode_compilation_queue().size();
}

static size_t& function_bytecode_compilation_worker_count()
{
    static size_t worker_count = 0;
    return worker_count;
}

static Threading::ConditionVariable& function_bytecode_compilation_queue_empty_condition()
{
    static Threading::ConditionVariable condition { function_bytecode_compilation_queue_mutex() };
    return condition;
}

static void drain_function_bytecode_compilation_queue()
{
    for (;;) {
        FunctionBytecodeCompilationJob job;
        {
            Threading::MutexLocker locker(function_bytecode_compilation_queue_mutex());
            auto& queue = function_bytecode_compilation_queue();
            auto& urgent_queue = urgent_function_bytecode_compilation_queue();
            if (!urgent_queue.is_empty()) {
                job = urgent_queue.dequeue();
            } else if (!queue.is_empty()) {
                job = queue.dequeue();
            } else {
                --function_bytecode_compilation_worker_count();
                if (function_bytecode_compilation_worker_count() == 0)
                    function_bytecode_compilation_queue_empty_condition().broadcast();
                return;
            }
        }

        job.input->compile_queued(job.source_len, job.builtin_abstract_operations_enabled);
    }
}

static void enqueue_function_bytecode_compilation(FunctionBytecodeInput& input, size_t source_len, bool builtin_abstract_operations_enabled, bool urgent)
{
    size_t workers_to_start = 0;
    {
        Threading::MutexLocker locker(function_bytecode_compilation_queue_mutex());
        FunctionBytecodeCompilationJob job {
            .input = RefPtr { input },
            .source_len = source_len,
            .builtin_abstract_operations_enabled = builtin_abstract_operations_enabled,
        };
        if (urgent)
            urgent_function_bytecode_compilation_queue().enqueue(move(job));
        else
            function_bytecode_compilation_queue().enqueue(move(job));

        auto target_worker_count = min(pending_function_bytecode_compilation_job_count(), Threading::ThreadPool::the().thread_count());
        while (function_bytecode_compilation_worker_count() < target_worker_count) {
            ++function_bytecode_compilation_worker_count();
            ++workers_to_start;
        }
    }

    for (size_t i = 0; i < workers_to_start; ++i)
        Threading::ThreadPool::the().submit([] { drain_function_bytecode_compilation_queue(); });
}

static void promote_function_bytecode_compilation(FunctionBytecodeInput& input, size_t source_len, bool builtin_abstract_operations_enabled)
{
    size_t workers_to_start = 0;
    {
        Threading::MutexLocker locker(function_bytecode_compilation_queue_mutex());
        // Leave the original queue entry in place: compile_queued() rechecks the input state, so the stale job becomes
        // a cheap no-op after this urgent job wins the race.
        urgent_function_bytecode_compilation_queue().enqueue({
            .input = RefPtr { input },
            .source_len = source_len,
            .builtin_abstract_operations_enabled = builtin_abstract_operations_enabled,
        });

        auto target_worker_count = min(pending_function_bytecode_compilation_job_count(), Threading::ThreadPool::the().thread_count());
        while (function_bytecode_compilation_worker_count() < target_worker_count) {
            ++function_bytecode_compilation_worker_count();
            ++workers_to_start;
        }
    }

    for (size_t i = 0; i < workers_to_start; ++i)
        Threading::ThreadPool::the().submit([] { drain_function_bytecode_compilation_queue(); });
}

void wait_for_background_bytecode_compilation()
{
    Threading::MutexLocker locker(function_bytecode_compilation_queue_mutex());
    function_bytecode_compilation_queue_empty_condition().wait_while([] {
        return function_bytecode_compilation_worker_count() > 0 || pending_function_bytecode_compilation_job_count() > 0;
    });
}

}

SharedFunctionInstanceData::SharedFunctionInstanceData(
    VM&,
    FunctionKind kind,
    Utf16FlyString name,
    i32 function_length,
    u32 formal_parameter_count,
    bool strict,
    bool is_arrow_function,
    bool has_simple_parameter_list,
    Vector<Utf16FlyString> parameter_names_for_mapped_arguments,
    void* rust_function_ast)
    : m_name(move(name))
    , m_function_length(function_length)
    , m_formal_parameter_count(formal_parameter_count)
    , m_parameter_names_for_mapped_arguments(move(parameter_names_for_mapped_arguments))
    , m_kind(kind)
    , m_strict(strict)
    , m_is_arrow_function(is_arrow_function)
    , m_has_simple_parameter_list(has_simple_parameter_list)
    , m_rust_function_ast(rust_function_ast)
    , m_use_rust_compilation(true)
{
    if (m_is_arrow_function)
        m_this_mode = ThisMode::Lexical;
    else if (m_strict)
        m_this_mode = ThisMode::Strict;
    else
        m_this_mode = ThisMode::Global;

    update_can_inline_call();
}

void SharedFunctionInstanceData::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_executable);
    for (auto& function : m_functions_to_initialize)
        visitor.visit(function.shared_data);
    m_class_field_initializer_name.visit([&](PropertyKey const& key) { key.visit_edges(visitor); }, [](auto&) {});
}

SharedFunctionInstanceData::~SharedFunctionInstanceData() = default;

void SharedFunctionInstanceData::set_executable(GC::Ptr<Bytecode::Executable> executable)
{
    m_executable = executable;
    update_can_inline_call();
}

void SharedFunctionInstanceData::set_is_class_constructor()
{
    m_is_class_constructor = true;
    update_can_inline_call();
}

void SharedFunctionInstanceData::update_asm_call_metadata()
{
    m_asm_call_metadata = m_formal_parameter_count;
    if (m_can_inline_call)
        m_asm_call_metadata |= asm_call_metadata_can_inline_call;
    if (m_function_environment_needed || m_this_value_needs_environment_resolution)
        m_asm_call_metadata |= asm_call_metadata_needs_environment_or_this_value_resolution;
    if (m_uses_this)
        m_asm_call_metadata |= asm_call_metadata_uses_this;
    if (m_strict)
        m_asm_call_metadata |= asm_call_metadata_strict;
}

void SharedFunctionInstanceData::finalize()
{
    Base::finalize();
    RustIntegration::free_function_ast(exchange(m_rust_function_ast, nullptr));
    m_bytecode_input = nullptr;
}

void SharedFunctionInstanceData::set_bytecode_input(RustIntegration::FunctionBytecodeInput& bytecode_input)
{
    RustIntegration::free_function_ast(exchange(m_rust_function_ast, nullptr));
    m_bytecode_input = adopt_ref(bytecode_input);
}

void SharedFunctionInstanceData::set_precompiled_bytecode(FFI::PrecompiledFunction* precompiled)
{
    VERIFY(precompiled);
    RustIntegration::free_function_ast(exchange(m_rust_function_ast, nullptr));
    if (!m_bytecode_input)
        m_bytecode_input = adopt_ref(*new RustIntegration::FunctionBytecodeInput(precompiled));
    else
        m_bytecode_input->set_precompiled(precompiled);
}

void SharedFunctionInstanceData::start_background_bytecode_compilation(bool builtin_abstract_operations_enabled)
{
    if (Bytecode::g_dump_bytecode || !m_background_bytecode_compilation_enabled || m_executable || !m_source_code)
        return;
    if (!m_bytecode_input) {
        if (!m_rust_function_ast)
            return;
        m_bytecode_input = adopt_ref(*new RustIntegration::FunctionBytecodeInput(exchange(m_rust_function_ast, nullptr)));
    }
    m_bytecode_input->start_background_compilation(m_source_code->length_in_code_units(), builtin_abstract_operations_enabled);
}

void* SharedFunctionInstanceData::take_rust_function_ast()
{
    return exchange(m_rust_function_ast, nullptr);
}

FFI::PrecompiledFunction* SharedFunctionInstanceData::take_precompiled_bytecode_or_compile(size_t source_len, bool builtin_abstract_operations_enabled)
{
    if (!m_bytecode_input)
        return nullptr;
    return m_bytecode_input->take_precompiled_or_compile(source_len, builtin_abstract_operations_enabled);
}

void SharedFunctionInstanceData::clear_compile_inputs()
{
    VERIFY(m_executable);
    m_functions_to_initialize.clear();
    m_var_names_to_initialize_binding.clear();
    m_lexical_bindings.clear();
    RustIntegration::free_function_ast(exchange(m_rust_function_ast, nullptr));
    m_bytecode_input = nullptr;
}

void SharedFunctionInstanceData::update_can_inline_call()
{
    m_can_inline_call = m_executable && m_kind == FunctionKind::Normal && !m_is_class_constructor;
    update_asm_call_metadata();
}

extern "C" void* rust_create_function_bytecode_input(void* rust_function_ast)
{
    return new RustIntegration::FunctionBytecodeInput(rust_function_ast);
}

extern "C" void rust_free_function_bytecode_input(void* bytecode_input)
{
    if (bytecode_input)
        static_cast<RustIntegration::FunctionBytecodeInput*>(bytecode_input)->unref();
}

extern "C" void rust_start_function_bytecode_input_background_compilation(
    void* bytecode_input, size_t source_len, bool builtin_abstract_operations_enabled)
{
    if (!Bytecode::g_dump_bytecode && bytecode_input)
        static_cast<RustIntegration::FunctionBytecodeInput*>(bytecode_input)
            ->start_background_compilation(source_len, builtin_abstract_operations_enabled);
}

}
