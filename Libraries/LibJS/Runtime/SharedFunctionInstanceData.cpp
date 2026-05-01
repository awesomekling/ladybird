/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/Runtime/SharedFunctionInstanceData.h>
#include <LibJS/RustIntegration.h>

namespace JS {

GC_DEFINE_ALLOCATOR(SharedFunctionInstanceData);

class SharedFunctionInstanceData::FunctionBytecodeInput {
public:
    explicit FunctionBytecodeInput(void* rust_function_ast)
        : m_input(rust_function_ast)
    {
    }

    explicit FunctionBytecodeInput(FFI::PrecompiledFunction* precompiled)
        : m_input(precompiled)
    {
    }

    ~FunctionBytecodeInput()
    {
        m_input.visit(
            [](Empty) {},
            [](void* rust_function_ast) { RustIntegration::free_function_ast(rust_function_ast); },
            [](FFI::PrecompiledFunction* precompiled) { RustIntegration::free_precompiled_function(precompiled); });
    }

    void set_precompiled(FFI::PrecompiledFunction* precompiled)
    {
        m_input = precompiled;
    }

    FFI::PrecompiledFunction* take_precompiled()
    {
        if (!m_input.has<FFI::PrecompiledFunction*>())
            return nullptr;
        auto* precompiled = m_input.get<FFI::PrecompiledFunction*>();
        m_input = Empty {};
        return precompiled;
    }

    void* take_rust_function_ast()
    {
        if (!m_input.has<void*>())
            return nullptr;
        auto* rust_function_ast = m_input.get<void*>();
        m_input = Empty {};
        return rust_function_ast;
    }

private:
    Variant<Empty, void*, FFI::PrecompiledFunction*> m_input;
};

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
    , m_use_rust_compilation(true)
{
    if (rust_function_ast)
        m_bytecode_input = make<FunctionBytecodeInput>(rust_function_ast);

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
    m_bytecode_input = nullptr;
}

void SharedFunctionInstanceData::set_precompiled_bytecode(FFI::PrecompiledFunction* precompiled)
{
    VERIFY(precompiled);
    if (!m_bytecode_input)
        m_bytecode_input = make<FunctionBytecodeInput>(precompiled);
    else
        m_bytecode_input->set_precompiled(precompiled);
}

FFI::PrecompiledFunction* SharedFunctionInstanceData::take_precompiled_bytecode()
{
    if (!m_bytecode_input)
        return nullptr;
    return m_bytecode_input->take_precompiled();
}

void* SharedFunctionInstanceData::take_rust_function_ast()
{
    if (!m_bytecode_input)
        return nullptr;
    return m_bytecode_input->take_rust_function_ast();
}

void SharedFunctionInstanceData::clear_compile_inputs()
{
    VERIFY(m_executable);
    m_functions_to_initialize.clear();
    m_var_names_to_initialize_binding.clear();
    m_lexical_bindings.clear();
    m_bytecode_input = nullptr;
}

void SharedFunctionInstanceData::update_can_inline_call()
{
    m_can_inline_call = m_executable && m_kind == FunctionKind::Normal && !m_is_class_constructor;
    update_asm_call_metadata();
}

}
