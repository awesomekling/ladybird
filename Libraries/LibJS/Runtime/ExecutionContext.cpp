/*
 * Copyright (c) 2020-2026, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, Linus Groh <linusg@serenityos.org>
 * Copyright (c) 2022, Luke Wilde <lukew@serenityos.org>
 * Copyright (c) 2024-2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGC/Heap.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Runtime/DeclarativeEnvironment.h>
#include <LibJS/Runtime/ExecutionContext.h>
#include <LibJS/Runtime/FunctionObject.h>

namespace JS {

class ExecutionContextAllocator {
public:
    NonnullOwnPtr<ExecutionContext> allocate(ReadonlySpan<Value> frame_template, u32 arguments_count)
    {
        auto tail_size = frame_template.size() + arguments_count;

        void* slot = nullptr;
        if (tail_size <= 4 && !m_execution_contexts_with_4_tail.is_empty()) {
            slot = m_execution_contexts_with_4_tail.take_last();
        } else if (tail_size <= 16 && !m_execution_contexts_with_16_tail.is_empty()) {
            slot = m_execution_contexts_with_16_tail.take_last();
        } else if (tail_size <= 64 && !m_execution_contexts_with_64_tail.is_empty()) {
            slot = m_execution_contexts_with_64_tail.take_last();
        } else if (tail_size <= 128 && !m_execution_contexts_with_128_tail.is_empty()) {
            slot = m_execution_contexts_with_128_tail.take_last();
        } else if (tail_size <= 256 && !m_execution_contexts_with_256_tail.is_empty()) {
            slot = m_execution_contexts_with_256_tail.take_last();
        } else if (tail_size <= 512 && !m_execution_contexts_with_512_tail.is_empty()) {
            slot = m_execution_contexts_with_512_tail.take_last();
        }

        if (slot) {
            return adopt_own(*new (slot) ExecutionContext(frame_template, arguments_count));
        }

        auto tail_allocation_size = [tail_size] -> u32 {
            if (tail_size <= 4)
                return 4;
            if (tail_size <= 16)
                return 16;
            if (tail_size <= 64)
                return 64;
            if (tail_size <= 128)
                return 128;
            if (tail_size <= 256)
                return 256;
            if (tail_size <= 512)
                return 512;
            return tail_size;
        };

        auto* memory = ::operator new(sizeof(ExecutionContext) + tail_allocation_size() * sizeof(Value));
        return adopt_own(*::new (memory) ExecutionContext(frame_template, arguments_count));
    }
    void deallocate(void* ptr, u32 tail_size)
    {
        if (tail_size <= 4) {
            m_execution_contexts_with_4_tail.append(ptr);
        } else if (tail_size <= 16) {
            m_execution_contexts_with_16_tail.append(ptr);
        } else if (tail_size <= 64) {
            m_execution_contexts_with_64_tail.append(ptr);
        } else if (tail_size <= 128) {
            m_execution_contexts_with_128_tail.append(ptr);
        } else if (tail_size <= 256) {
            m_execution_contexts_with_256_tail.append(ptr);
        } else if (tail_size <= 512) {
            m_execution_contexts_with_512_tail.append(ptr);
        } else {
            ::operator delete(ptr);
        }
    }

private:
    Vector<void*> m_execution_contexts_with_4_tail;
    Vector<void*> m_execution_contexts_with_16_tail;
    Vector<void*> m_execution_contexts_with_64_tail;
    Vector<void*> m_execution_contexts_with_128_tail;
    Vector<void*> m_execution_contexts_with_256_tail;
    Vector<void*> m_execution_contexts_with_512_tail;
};

static NeverDestroyed<ExecutionContextAllocator> s_execution_context_allocator;

NonnullOwnPtr<ExecutionContext> ExecutionContext::create(ReadonlySpan<Value> frame_template, u32 arguments_count)
{
    return s_execution_context_allocator->allocate(frame_template, arguments_count);
}

void ExecutionContext::operator delete(void* ptr)
{
    auto const* execution_context = static_cast<ExecutionContext const*>(ptr);
    s_execution_context_allocator->deallocate(ptr, execution_context->registers_and_constants_and_locals_and_arguments_count);
}

NonnullOwnPtr<ExecutionContext> ExecutionContext::copy() const
{
    // NB: We hand our own non-argument slots over as the template, so the new
    //     frame starts out as a verbatim copy of ours.
    auto non_argument_count = registers_and_constants_and_locals_and_arguments_count - argument_count;
    auto copy = create({ registers_and_constants_and_locals_and_arguments(), non_argument_count }, argument_count);
    copy->function = function;
    copy->realm = realm;
    copy->script_or_module = script_or_module;
    copy->lexical_environment = lexical_environment;
    copy->variable_environment = variable_environment;
    copy->private_environment = private_environment;
    copy->program_counter = program_counter;
    copy->yield_continuation = yield_continuation;
    copy->yield_is_await = yield_is_await;
    copy->yield_value_is_iterator_result = yield_value_is_iterator_result;
    copy->caller_is_construct = caller_is_construct;
    copy->this_value = this_value;
    copy->executable = executable;
    copy->passed_argument_count = passed_argument_count;
    // NB: The template copy above already brought over everything but the arguments.
    for (size_t i = 0; i < argument_count; ++i)
        copy->arguments_data()[i] = arguments_data()[i];
    copy->argument_count = argument_count;
    return copy;
}

void ExecutionContext::visit_edges(Cell::Visitor& visitor)
{
    visitor.visit(function);
    visitor.visit(realm);
    visitor.visit(variable_environment);
    visitor.visit(lexical_environment);
    visitor.visit(private_environment);
    visitor.visit(this_value);
    visitor.visit(executable);
    visitor.visit(registers_and_constants_and_locals_and_arguments_span());
    visitor.visit(script_or_module);
}

}
