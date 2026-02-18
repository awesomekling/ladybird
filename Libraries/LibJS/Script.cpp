/*
 * Copyright (c) 2021, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGC/DeferGC.h>
#include <LibJS/AST.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/Generator.h>
#include <LibJS/BytecodeFactory.h>
#include <LibJS/Lexer.h>
#include <LibJS/Parser.h>
#include <LibJS/PipelineComparison.h>
#include <LibJS/Runtime/ECMAScriptFunctionObject.h>
#include <LibJS/Runtime/GlobalEnvironment.h>
#include <LibJS/Runtime/SharedFunctionInstanceData.h>
#include <LibJS/Runtime/VM.h>
#include <LibJS/Script.h>
#include <LibJS/SourceCode.h>

namespace JS {

bool g_dump_ast = false;
bool g_dump_ast_use_color = false;

GC_DEFINE_ALLOCATOR(Script);

#ifdef ENABLE_RUST_PARSER
// Builder populated by Rust via callbacks during rust_compile_script.
struct ScriptGdiBuilder {
    GC::Ptr<Bytecode::Executable> executable;
    bool is_strict_mode { false };
    Vector<Utf16FlyString> lexical_names;
    Vector<Utf16FlyString> var_names;
    Vector<Script::FunctionToInitialize> functions_to_initialize;
    HashTable<Utf16FlyString> declared_function_names;
    Vector<Utf16FlyString> var_scoped_names;
    Vector<Utf16FlyString> annex_b_candidate_names;
    Vector<Script::LexicalBinding> lexical_bindings;
};
#endif

}

#ifdef ENABLE_RUST_PARSER

static Utf16FlyString utf16_fly_from(uint16_t const* data, size_t len)
{
    return Utf16FlyString::from_utf16(Utf16View { reinterpret_cast<char16_t const*>(data), len });
}

extern "C" void script_gdi_push_lexical_name(void* ctx, uint16_t const* name, size_t len)
{
    static_cast<JS::ScriptGdiBuilder*>(ctx)->lexical_names.append(utf16_fly_from(name, len));
}

extern "C" void script_gdi_push_var_name(void* ctx, uint16_t const* name, size_t len)
{
    static_cast<JS::ScriptGdiBuilder*>(ctx)->var_names.append(utf16_fly_from(name, len));
}

extern "C" void script_gdi_push_function(void* ctx, void* sfd_ptr, uint16_t const* name, size_t len)
{
    auto& builder = *static_cast<JS::ScriptGdiBuilder*>(ctx);
    auto fn_name = utf16_fly_from(name, len);
    builder.declared_function_names.set(fn_name);
    auto& sfd = *static_cast<JS::SharedFunctionInstanceData*>(sfd_ptr);
    builder.functions_to_initialize.append({ sfd, move(fn_name) });
}

extern "C" void script_gdi_push_var_scoped_name(void* ctx, uint16_t const* name, size_t len)
{
    static_cast<JS::ScriptGdiBuilder*>(ctx)->var_scoped_names.append(utf16_fly_from(name, len));
}

extern "C" void script_gdi_push_annex_b_name(void* ctx, uint16_t const* name, size_t len)
{
    static_cast<JS::ScriptGdiBuilder*>(ctx)->annex_b_candidate_names.append(utf16_fly_from(name, len));
}

extern "C" void script_gdi_push_lexical_binding(void* ctx, uint16_t const* name, size_t len, bool is_constant)
{
    static_cast<JS::ScriptGdiBuilder*>(ctx)->lexical_bindings.append({ utf16_fly_from(name, len), is_constant });
}

static void collect_rust_parse_error(void* ctx, char const* message, size_t message_len, uint32_t line, uint32_t column)
{
    auto& errors = *static_cast<Vector<JS::ParserError>*>(ctx);
    errors.append({
        MUST(String::from_utf8({ message, message_len })),
        JS::Position { line, column, 0 },
    });
}

#endif // ENABLE_RUST_PARSER

namespace JS {

// 16.1.5 ParseScript ( sourceText, realm, hostDefined ), https://tc39.es/ecma262/#sec-parse-script
Result<GC::Ref<Script>, Vector<ParserError>> Script::parse(StringView source_text, Realm& realm, StringView filename, HostDefined* host_defined, size_t line_number_offset)
{
#ifdef ENABLE_RUST_PARSER
    static bool const use_rust_codegen = getenv("USE_RUST_CODEGEN") != nullptr;
    bool const compare_pipelines = compare_pipelines_enabled();

    if (use_rust_codegen || compare_pipelines) {
        auto source_code = SourceCode::create(
            String::from_utf8(filename).release_value_but_fixme_should_propagate_errors(),
            Utf16String::from_utf8(source_text));

        auto const& code_view = source_code->code_view();
        auto length = code_view.length_in_code_units();

        GC::DeferGC defer_gc(realm.vm().heap());
        ScriptGdiBuilder builder;
        Vector<ParserError> parse_errors;

        u8* rust_ast_data = nullptr;
        size_t rust_ast_len = 0;
        u8** rust_ast_data_ptr = compare_pipelines ? &rust_ast_data : nullptr;
        size_t* rust_ast_len_ptr = compare_pipelines ? &rust_ast_len : nullptr;

        void* exec_ptr;
        if (code_view.has_ascii_storage()) {
            auto ascii = code_view.ascii_span();
            Vector<u16> utf16_buf;
            utf16_buf.ensure_capacity(length);
            for (size_t i = 0; i < length; ++i)
                utf16_buf.unchecked_append(static_cast<u16>(ascii[i]));
            exec_ptr = rust_compile_script(utf16_buf.data(), length, &realm.vm(), source_code.ptr(), &builder, g_dump_ast, g_dump_ast_use_color,
                &parse_errors, collect_rust_parse_error, rust_ast_data_ptr, rust_ast_len_ptr);
        } else {
            auto utf16 = code_view.utf16_span();
            exec_ptr = rust_compile_script(reinterpret_cast<u16 const*>(utf16.data()), length, &realm.vm(), source_code.ptr(), &builder, g_dump_ast, g_dump_ast_use_color,
                &parse_errors, collect_rust_parse_error, rust_ast_data_ptr, rust_ast_len_ptr);
        }

        if (!exec_ptr) {
            if (rust_ast_data)
                rust_free_string(rust_ast_data, rust_ast_len);
            return parse_errors;
        }

        if (compare_pipelines) {
            auto rust_ast_dump = StringView { rust_ast_data, rust_ast_len };

            // Run C++ pipeline for comparison.
            auto parser = Parser(Lexer(source_code, line_number_offset));
            auto cpp_program = parser.parse_program();

            if (!parser.has_errors()) {
                // Compare AST dumps.
                auto cpp_ast_dump = cpp_program->dump_to_string();
                compare_pipeline_asts(rust_ast_dump, cpp_ast_dump, filename);

                // Run Annex B analysis on the C++ AST before generating bytecode.
                // Normally this runs at runtime during GDI, but the Rust pipeline
                // handles it statically during scope analysis.
                if (!cpp_program->is_strict_mode()) {
                    HashTable<Utf16FlyString> lexical_names;
                    MUST(cpp_program->for_each_lexically_declared_identifier([&](Identifier const& identifier) -> ThrowCompletionOr<void> {
                        lexical_names.set(identifier.string());
                        return {};
                    }));
                    MUST(cpp_program->for_each_function_hoistable_with_annexB_extension([&](FunctionDeclaration& function_declaration) -> ThrowCompletionOr<void> {
                        if (!lexical_names.contains(function_declaration.name()))
                            function_declaration.set_should_do_additional_annexB_steps();
                        return {};
                    }));
                }

                // Compare bytecode dumps.
                auto& rust_executable = *static_cast<Bytecode::Executable*>(exec_ptr);
                auto cpp_executable = Bytecode::Generator::generate_from_ast_node(realm.vm(), *cpp_program, {});
                auto rust_bytecode_dump = rust_executable.dump_to_string();
                auto cpp_bytecode_dump = cpp_executable->dump_to_string();
                compare_pipeline_bytecode(rust_bytecode_dump, cpp_bytecode_dump, filename);
            }

            rust_free_string(rust_ast_data, rust_ast_len);
        }

        auto& executable = *static_cast<Bytecode::Executable*>(exec_ptr);
        return realm.heap().allocate<Script>(realm, filename, move(builder), executable, host_defined);
    }
#endif

    // 1. Let script be ParseText(sourceText, Script).
    auto parser = Parser(Lexer(SourceCode::create(String::from_utf8(filename).release_value_but_fixme_should_propagate_errors(), Utf16String::from_utf8(source_text)), line_number_offset));
    auto script = parser.parse_program();

    // 2. If script is a List of errors, return body.
    if (parser.has_errors())
        return parser.errors();

    // 3. Return Script Record { [[Realm]]: realm, [[ECMAScriptCode]]: script, [[HostDefined]]: hostDefined }.
    return realm.heap().allocate<Script>(realm, filename, move(script), host_defined);
}

Script::Script(Realm& realm, StringView filename, RefPtr<Program> parse_node, HostDefined* host_defined)
    : m_realm(realm)
    , m_parse_node(move(parse_node))
    , m_filename(filename)
    , m_host_defined(host_defined)
{
    auto& vm = realm.vm();
    auto& program = *m_parse_node;

    m_is_strict_mode = program.is_strict_mode();

    // Pre-compute lexically declared names (GDI step 3).
    MUST(program.for_each_lexically_declared_identifier([&](Identifier const& identifier) -> ThrowCompletionOr<void> {
        m_lexical_names.append(identifier.string());
        return {};
    }));

    // Pre-compute var declared names (GDI step 4).
    MUST(program.for_each_var_declared_identifier([&](Identifier const& identifier) -> ThrowCompletionOr<void> {
        m_var_names.append(identifier.string());
        return {};
    }));

    // Pre-compute functions to initialize and declared function names (GDI steps 7-8).
    MUST(program.for_each_var_function_declaration_in_reverse_order([&](FunctionDeclaration const& function) -> ThrowCompletionOr<void> {
        auto function_name = function.name();
        if (m_declared_function_names.set(function_name) != AK::HashSetResult::InsertedNewEntry)
            return {};
        m_functions_to_initialize.append({ SharedFunctionInstanceData::create_for_function_node(vm, function), function_name });
        return {};
    }));

    // Pre-compute var scoped variable names (GDI step 10).
    MUST(program.for_each_var_scoped_variable_declaration([&](VariableDeclaration const& declaration) {
        return declaration.for_each_bound_identifier([&](Identifier const& identifier) -> ThrowCompletionOr<void> {
            m_var_scoped_names.append(identifier.string());
            return {};
        });
    }));

    // Pre-compute AnnexB candidates (GDI step 13).
    if (!m_is_strict_mode) {
        MUST(program.for_each_function_hoistable_with_annexB_extension([&](FunctionDeclaration& function_declaration) -> ThrowCompletionOr<void> {
            m_annex_b_candidate_names.append(function_declaration.name());
            m_annex_b_function_declarations.append(function_declaration);
            return {};
        }));
    }

    // Pre-compute lexical bindings (GDI step 15).
    MUST(program.for_each_lexically_scoped_declaration([&](Declaration const& declaration) {
        return declaration.for_each_bound_identifier([&](Identifier const& identifier) -> ThrowCompletionOr<void> {
            m_lexical_bindings.append({ identifier.string(), declaration.is_constant_declaration() });
            return {};
        });
    }));
}

#ifdef ENABLE_RUST_PARSER
Script::Script(Realm& realm, StringView filename, ScriptGdiBuilder&& builder, Bytecode::Executable& executable, HostDefined* host_defined)
    : m_realm(realm)
    , m_executable(&executable)
    , m_lexical_names(move(builder.lexical_names))
    , m_var_names(move(builder.var_names))
    , m_functions_to_initialize(move(builder.functions_to_initialize))
    , m_declared_function_names(move(builder.declared_function_names))
    , m_var_scoped_names(move(builder.var_scoped_names))
    , m_annex_b_candidate_names(move(builder.annex_b_candidate_names))
    , m_lexical_bindings(move(builder.lexical_bindings))
    , m_is_strict_mode(builder.is_strict_mode)
    , m_filename(filename)
    , m_host_defined(host_defined)
{
}
#endif

// 16.1.7 GlobalDeclarationInstantiation ( script, env ), https://tc39.es/ecma262/#sec-globaldeclarationinstantiation
ThrowCompletionOr<void> Script::global_declaration_instantiation(VM& vm, GlobalEnvironment& global_environment)
{
    auto& realm = *vm.current_realm();

    // 1. Let lexNames be the LexicallyDeclaredNames of script.
    // 2. Let varNames be the VarDeclaredNames of script.
    // 3. For each element name of lexNames, do
    for (auto const& name : m_lexical_names) {
        // a. If env.HasLexicalDeclaration(name) is true, throw a SyntaxError exception.
        if (global_environment.has_lexical_declaration(name))
            return vm.throw_completion<SyntaxError>(ErrorType::TopLevelVariableAlreadyDeclared, name);

        // b. Let hasRestrictedGlobal be ? HasRestrictedGlobalProperty(env, name).
        auto has_restricted_global = TRY(global_environment.has_restricted_global_property(name));

        // d. If hasRestrictedGlobal is true, throw a SyntaxError exception.
        if (has_restricted_global)
            return vm.throw_completion<SyntaxError>(ErrorType::RestrictedGlobalProperty, name);
    }

    // 4. For each element name of varNames, do
    for (auto const& name : m_var_names) {
        // a. If env.HasLexicalDeclaration(name) is true, throw a SyntaxError exception.
        if (global_environment.has_lexical_declaration(name))
            return vm.throw_completion<SyntaxError>(ErrorType::TopLevelVariableAlreadyDeclared, name);
    }

    // 5. Let varDeclarations be the VarScopedDeclarations of script.
    // 6. Let functionsToInitialize be a new empty List.
    // 7. Let declaredFunctionNames be a new empty List.
    // 8. For each element d of varDeclarations, in reverse List order, do
    for (auto const& function : m_functions_to_initialize) {
        // 1. Let fnDefinable be ? env.CanDeclareGlobalFunction(fn).
        auto function_definable = TRY(global_environment.can_declare_global_function(function.name));

        // 2. If fnDefinable is false, throw a TypeError exception.
        if (!function_definable)
            return vm.throw_completion<TypeError>(ErrorType::CannotDeclareGlobalFunction, function.name);
    }

    // 9. Let declaredVarNames be a new empty List.
    HashTable<Utf16FlyString> declared_var_names;

    // 10. For each element d of varDeclarations, do
    for (auto const& name : m_var_scoped_names) {
        // 1. If vn is not an element of declaredFunctionNames, then
        if (m_declared_function_names.contains(name))
            continue;

        // a. Let vnDefinable be ? env.CanDeclareGlobalVar(vn).
        auto var_definable = TRY(global_environment.can_declare_global_var(name));

        // b. If vnDefinable is false, throw a TypeError exception.
        if (!var_definable)
            return vm.throw_completion<TypeError>(ErrorType::CannotDeclareGlobalVariable, name);

        // c. If vn is not an element of declaredVarNames, then
        // i. Append vn to declaredVarNames.
        declared_var_names.set(name);
    }

    // 12. NOTE: Annex B.3.2.2 adds additional steps at this point.
    // 12. Let strict be IsStrict of script.
    // 13. If strict is false, then
    if (!m_is_strict_mode) {
        // a. Let declaredFunctionOrVarNames be the list-concatenation of declaredFunctionNames and declaredVarNames.
        // b. For each FunctionDeclaration f that is directly contained in the StatementList of a Block, CaseClause, or DefaultClause Contained within script, do
        for (size_t i = 0; i < m_annex_b_candidate_names.size(); ++i) {
            // i. Let F be StringValue of the BindingIdentifier of f.
            auto& function_name = m_annex_b_candidate_names[i];

            // 1. If env.HasLexicalDeclaration(F) is false, then
            if (global_environment.has_lexical_declaration(function_name))
                continue;

            // a. Let fnDefinable be ? env.CanDeclareGlobalVar(F).
            auto function_definable = TRY(global_environment.can_declare_global_function(function_name));
            // b. If fnDefinable is true, then
            if (!function_definable)
                continue;

            // ii. If declaredFunctionOrVarNames does not contain F, then
            if (!m_declared_function_names.contains(function_name) && !declared_var_names.contains(function_name)) {
                // i. Perform ? env.CreateGlobalVarBinding(F, false).
                TRY(global_environment.create_global_var_binding(function_name, false));
            }

            // iii. When the FunctionDeclaration f is evaluated, perform the following steps in place of the FunctionDeclaration Evaluation algorithm provided in 15.2.6:
            if (i < m_annex_b_function_declarations.size())
                m_annex_b_function_declarations[i]->set_should_do_additional_annexB_steps();
        }
    }

    // 14. Let privateEnv be null.
    PrivateEnvironment* private_environment = nullptr;

    // 15. For each element d of lexDeclarations, do
    for (auto const& binding : m_lexical_bindings) {
        // i. If IsConstantDeclaration of d is true, then
        if (binding.is_constant) {
            // 1. Perform ? env.CreateImmutableBinding(dn, true).
            TRY(global_environment.create_immutable_binding(vm, binding.name, true));
        }
        // ii. Else,
        else {
            // 1. Perform ? env.CreateMutableBinding(dn, false).
            TRY(global_environment.create_mutable_binding(vm, binding.name, false));
        }
    }

    // 16. For each Parse Node f of functionsToInitialize, do
    // NB: We iterate in reverse order since we appended the functions
    //     instead of prepending during pre-computation.
    for (auto const& function_to_initialize : m_functions_to_initialize.in_reverse()) {
        // a. Let fn be the sole element of the BoundNames of f.
        // b. Let fo be InstantiateFunctionObject of f with arguments env and privateEnv.
        auto function = ECMAScriptFunctionObject::create_from_function_data(
            realm,
            function_to_initialize.shared_data,
            &global_environment,
            private_environment);

        // c. Perform ? env.CreateGlobalFunctionBinding(fn, fo, false).
        TRY(global_environment.create_global_function_binding(function->name(), function, false));
    }

    // 17. For each String vn of declaredVarNames, do
    for (auto& var_name : declared_var_names) {
        // a. Perform ? env.CreateGlobalVarBinding(vn, false).
        TRY(global_environment.create_global_var_binding(var_name, false));
    }

    // 18. Return unused.
    return {};
}

void Script::drop_ast()
{
    m_parse_node = nullptr;
    m_annex_b_function_declarations.clear();
}

Script::~Script()
{
}

void Script::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_realm);
    visitor.visit(m_executable);
    for (auto const& function : m_functions_to_initialize)
        visitor.visit(function.shared_data);
    if (m_host_defined)
        m_host_defined->visit_host_defined_self(visitor);
    for (auto const& loaded_module : m_loaded_modules)
        visitor.visit(loaded_module.module);
}

}
