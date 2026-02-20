/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashTable.h>
#include <AK/Optional.h>
#include <AK/Result.h>
#include <AK/Utf16FlyString.h>
#include <LibGC/Ptr.h>
#include <LibGC/Root.h>
#include <LibJS/Forward.h>
#include <LibJS/ParserError.h>
#include <LibJS/Runtime/AbstractOperations.h>
#include <LibJS/Runtime/FunctionKind.h>
#include <LibJS/Script.h>
#include <LibJS/SourceTextModule.h>

namespace JS::RustIntegration {

// Result type for compile_script().
struct ScriptResult {
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

// Result type for compile_eval() and compile_shadow_realm_eval().
struct EvalResult {
    GC::Ptr<Bytecode::Executable> executable;
    bool is_strict_mode { false };
    EvalDeclarationData declaration_data;
};

// Result type for compile_module().
struct ModuleResult {
    bool has_top_level_await { false };
    Vector<ModuleRequest> requested_modules;
    Vector<ImportEntry> import_entries;
    Vector<ExportEntry> local_export_entries;
    Vector<ExportEntry> indirect_export_entries;
    Vector<ExportEntry> star_export_entries;
    Optional<Utf16FlyString> default_export_binding_name;
    Vector<Utf16FlyString> var_declared_names;
    Vector<SourceTextModule::LexicalBinding> lexical_bindings;
    Vector<SourceTextModule::FunctionToInitialize> functions_to_initialize;
    GC::Ptr<Bytecode::Executable> executable;
    GC::Ptr<SharedFunctionInstanceData> tla_shared_data;
};

// Compile a script. Returns nullopt if Rust is not available.
Optional<Result<ScriptResult, Vector<ParserError>>> compile_script(
    StringView source_text, Realm& realm, StringView filename, size_t line_number_offset);

// Compile eval code. Returns nullopt if Rust is not available.
// On success, the executable's name is set to "eval".
Optional<Result<EvalResult, String>> compile_eval(
    PrimitiveString& code_string, VM& vm,
    CallerMode strict_caller, bool in_function, bool in_method,
    bool in_derived_constructor, bool in_class_field_initializer);

// Compile ShadowRealm eval code. Returns nullopt if Rust is not available.
// On success, the executable's name is set to "ShadowRealmEval".
Optional<Result<EvalResult, String>> compile_shadow_realm_eval(
    PrimitiveString& source_text, VM& vm);

// Compile a module. Returns nullopt if Rust is not available.
Optional<Result<ModuleResult, Vector<ParserError>>> compile_module(
    StringView source_text, Realm& realm, StringView filename);

// Compile a dynamic function (new Function()). Returns nullopt if Rust is not available.
// On success, returns a SharedFunctionInstanceData with source_text set.
Optional<Result<GC::Ref<SharedFunctionInstanceData>, String>> compile_dynamic_function(
    VM& vm, StringView source_text, StringView parameters_string, StringView body_parse_string,
    FunctionKind kind);

// Compile a builtin JS file. Returns nullopt if Rust is not available.
Optional<Vector<GC::Root<SharedFunctionInstanceData>>> compile_builtin_file(
    unsigned char const* script_text, VM& vm);

// Compile a function body for lazy compilation (NativeJavaScriptBackedFunction).
// Returns nullptr if Rust is not available or the SFD doesn't use Rust compilation.
GC::Ptr<Bytecode::Executable> compile_function(VM& vm, SharedFunctionInstanceData& shared_data);

}
