/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/ByteString.h>
#include <AK/FlyString.h>
#include <AK/NonnullRefPtr.h>
#include <AK/Utf16FlyString.h>
#include <AK/Utf16String.h>
#include <AK/Utf16View.h>
#include <AK/Vector.h>
#include <LibJS/AST.h>
#include <LibJS/ASTFactory.h>
#include <LibJS/Runtime/ModuleRequest.h>
#include <LibJS/Runtime/RegExpObject.h>
#include <LibJS/SourceCode.h>
#include <LibJS/SourceRange.h>
#include <LibRegex/Regex.h>

using namespace JS;

static Utf16View make_utf16_view(u16 const* data, size_t length)
{
    return Utf16View(reinterpret_cast<char16_t const*>(data), length);
}

// Arena holds NonnullRefPtr<ASTNode const> to prevent premature deallocation.
struct ASTArena {
    Vector<NonnullRefPtr<ASTNode const>> nodes;
    Vector<NonnullRefPtr<FunctionParameters const>> parameters;
    Vector<NonnullRefPtr<BindingPattern const>> binding_patterns;
};

static SourceRange make_range(SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& code = *static_cast<SourceCode const*>(source_code);
    return SourceRange {
        code,
        Position { start_line, start_column, start_offset },
        Position { end_line, end_column, end_offset },
    };
}

static ASTNodeHandle arena_add(ASTArena& arena, NonnullRefPtr<ASTNode const> node)
{
    auto* raw = const_cast<ASTNode*>(node.ptr());
    arena.nodes.append(move(node));
    return static_cast<ASTNodeHandle>(raw);
}

template<typename T>
static T& as_node(ASTNodeHandle handle)
{
    return *static_cast<T*>(handle);
}

template<typename T>
static NonnullRefPtr<T const> as_ref(ASTNodeHandle handle)
{
    return *static_cast<T const*>(handle);
}

template<typename T>
static NonnullRefPtr<T> as_mut_ref(ASTNodeHandle handle)
{
    return *static_cast<T*>(handle);
}

static FlyString make_fly_string(u16 const* data, size_t length)
{
    auto view = make_utf16_view(data, length);
    auto utf8 = view.to_utf8_but_should_be_ported_to_utf16();
    return MUST(FlyString::from_utf8(utf8.bytes_as_string_view()));
}

template<typename T>
static RefPtr<T const> as_nullable_ref(ASTNodeHandle handle)
{
    if (!handle)
        return nullptr;
    return as_ref<T>(handle);
}

extern "C" {

ASTArenaHandle ast_arena_create()
{
    return new ASTArena();
}

void ast_arena_destroy(ASTArenaHandle arena)
{
    delete static_cast<ASTArena*>(arena);
}

void ast_node_ref(ASTNodeHandle handle)
{
    static_cast<ASTNode*>(handle)->ref();
}

// === Program / ScopeNode ===

ASTNodeHandle ast_create_program(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 program_type)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto type = program_type == 0 ? Program::Type::Script : Program::Type::Module;
    return arena_add(arena, create_ast_node<Program>(range, type));
}

ASTNodeHandle ast_create_block_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<BlockStatement>(range));
}

ASTNodeHandle ast_create_function_body(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<FunctionBody>(range));
}

void ast_scope_node_append(ASTNodeHandle scope_node, ASTNodeHandle statement)
{
    auto& scope = as_node<ScopeNode>(scope_node);
    scope.append(as_ref<Statement>(statement));
}

void ast_scope_node_set_strict_mode(ASTNodeHandle scope_node)
{
    // set_strict_mode is on Program and FunctionBody, not ScopeNode.
    // We try Program first, then FunctionBody.
    auto* node = static_cast<ASTNode*>(scope_node);
    if (is<Program>(*node))
        static_cast<Program&>(*node).set_strict_mode();
    else if (is<FunctionBody>(*node))
        static_cast<FunctionBody&>(*node).set_strict_mode();
}

// === Literals ===

ASTNodeHandle ast_create_numeric_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    double value)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<NumericLiteral>(range, value));
}

ASTNodeHandle ast_create_string_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* value, size_t value_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto str = Utf16String::from_utf16(make_utf16_view(value, value_len));
    return arena_add(arena, create_ast_node<StringLiteral>(range, move(str)));
}

ASTNodeHandle ast_create_boolean_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    bool value)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<BooleanLiteral>(range, value));
}

ASTNodeHandle ast_create_null_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<NullLiteral>(range));
}

ASTNodeHandle ast_create_bigint_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    char const* value, size_t value_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<BigIntLiteral>(range, ByteString(StringView { value, value_len })));
}

ASTNodeHandle ast_create_regexp_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* pattern, size_t pattern_len,
    u16 const* flags, size_t flags_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);

    auto pattern_utf16 = Utf16String::from_utf16(make_utf16_view(pattern, pattern_len));
    auto flags_utf16 = Utf16String::from_utf16(make_utf16_view(flags, flags_len));

    auto parsed_flags = JS::RegExpObject::default_flags;
    if (flags_len > 0) {
        auto parsed_flags_or_error = regex_flags_from_string(flags_utf16.utf16_view());
        if (!parsed_flags_or_error.is_error())
            parsed_flags = parsed_flags_or_error.release_value();
    }

    String parsed_pattern;
    auto parsed_pattern_result = parse_regex_pattern(pattern_utf16.utf16_view(), parsed_flags.has_flag_set(ECMAScriptFlags::Unicode), parsed_flags.has_flag_set(ECMAScriptFlags::UnicodeSets));
    if (!parsed_pattern_result.is_error())
        parsed_pattern = parsed_pattern_result.release_value();

    auto parsed_regex = Regex<ECMA262>::parse_pattern(parsed_pattern, parsed_flags);

    if (parsed_regex.error != regex::Error::NoError) {
        // If the regex failed to compile, use an empty pattern to avoid VERIFY failures at runtime.
        parsed_pattern = ""_string;
        parsed_regex = Regex<ECMA262>::parse_pattern(parsed_pattern, parsed_flags);
    }

    return arena_add(arena, create_ast_node<RegExpLiteral>(range, move(parsed_regex), move(parsed_pattern), parsed_flags, move(pattern_utf16), move(flags_utf16)));
}

// === Identifiers ===

ASTNodeHandle ast_create_identifier(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* name, size_t name_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto fly = Utf16FlyString::from_utf16(make_utf16_view(name, name_len));
    return arena_add(arena, create_ast_node<Identifier>(range, move(fly)));
}

ASTNodeHandle ast_create_private_identifier(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* name, size_t name_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto fly = Utf16FlyString::from_utf16(make_utf16_view(name, name_len));
    return arena_add(arena, create_ast_node<PrivateIdentifier>(range, move(fly)));
}

void ast_identifier_set_local_variable_index(ASTNodeHandle identifier_handle, u32 index)
{
    auto& identifier = static_cast<Identifier&>(*static_cast<ASTNode*>(identifier_handle));
    identifier.set_local_variable_index(index);
}

void ast_identifier_set_argument_index(ASTNodeHandle identifier_handle, u32 index)
{
    auto& identifier = static_cast<Identifier&>(*static_cast<ASTNode*>(identifier_handle));
    identifier.set_argument_index(index);
}

u32 ast_scope_node_add_local_variable(ASTNodeHandle scope_handle, u16 const* name, size_t name_len, u8 declaration_kind)
{
    auto& scope_node = static_cast<ScopeNode&>(*static_cast<ASTNode*>(scope_handle));
    auto fly = Utf16FlyString::from_utf16(make_utf16_view(name, name_len));
    auto kind = static_cast<LocalVariable::DeclarationKind>(declaration_kind);
    return scope_node.add_local_variable(move(fly), kind);
}

void ast_identifier_set_is_global(ASTNodeHandle identifier_handle)
{
    auto& identifier = static_cast<Identifier&>(*static_cast<ASTNode*>(identifier_handle));
    identifier.set_is_global();
}

void ast_identifier_set_is_inside_scope_with_eval(ASTNodeHandle identifier_handle)
{
    auto& identifier = static_cast<Identifier&>(*static_cast<ASTNode*>(identifier_handle));
    identifier.set_is_inside_scope_with_eval();
}

void ast_identifier_set_declaration_kind(ASTNodeHandle identifier_handle, u8 kind)
{
    auto& identifier = static_cast<Identifier&>(*static_cast<ASTNode*>(identifier_handle));
    identifier.set_declaration_kind(static_cast<DeclarationKind>(kind));
}

bool ast_identifier_is_local(ASTNodeHandle identifier_handle)
{
    auto const& identifier = static_cast<Identifier const&>(*static_cast<ASTNode const*>(identifier_handle));
    return identifier.is_local();
}

bool ast_identifier_is_inside_scope_with_eval(ASTNodeHandle identifier_handle)
{
    auto const& identifier = static_cast<Identifier const&>(*static_cast<ASTNode const*>(identifier_handle));
    return identifier.is_inside_scope_with_eval();
}

// === Expressions ===

ASTNodeHandle ast_create_this_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ThisExpression>(range));
}

ASTNodeHandle ast_create_super_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<SuperExpression>(range));
}

ASTNodeHandle ast_create_binary_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle lhs, ASTNodeHandle rhs)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<BinaryExpression>(range,
        static_cast<BinaryOp>(op), as_ref<Expression>(lhs), as_ref<Expression>(rhs)));
}

ASTNodeHandle ast_create_logical_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle lhs, ASTNodeHandle rhs)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<LogicalExpression>(range,
        static_cast<LogicalOp>(op), as_ref<Expression>(lhs), as_ref<Expression>(rhs)));
}

ASTNodeHandle ast_create_unary_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle operand)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<UnaryExpression>(range,
        static_cast<UnaryOp>(op), as_ref<Expression>(operand)));
}

ASTNodeHandle ast_create_update_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle argument, bool prefixed)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<UpdateExpression>(range,
        static_cast<UpdateOp>(op), as_ref<Expression>(argument), prefixed));
}

ASTNodeHandle ast_create_assignment_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle lhs, ASTNodeHandle rhs)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<AssignmentExpression>(range,
        static_cast<AssignmentOp>(op), as_ref<Expression>(lhs), as_ref<Expression>(rhs)));
}

ASTNodeHandle ast_create_conditional_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle test, ASTNodeHandle consequent, ASTNodeHandle alternate)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ConditionalExpression>(range,
        as_ref<Expression>(test), as_ref<Expression>(consequent), as_ref<Expression>(alternate)));
}

ASTNodeHandle ast_create_sequence_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* expressions, size_t count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<Expression const>> exprs;
    exprs.ensure_capacity(count);
    for (size_t i = 0; i < count; ++i)
        exprs.unchecked_append(as_ref<Expression>(expressions[i]));
    return arena_add(arena, create_ast_node<SequenceExpression>(range, move(exprs)));
}

ASTNodeHandle ast_create_member_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle object, ASTNodeHandle property, bool computed)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<MemberExpression>(range,
        as_ref<Expression>(object), as_ref<Expression>(property), computed));
}

ASTNodeHandle ast_create_call_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle callee,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<CallExpression::Argument> args;
    args.ensure_capacity(argument_count);
    for (size_t i = 0; i < argument_count; ++i) {
        args.unchecked_append({
            .value = as_ref<Expression>(argument_values[i]),
            .is_spread = argument_is_spread[i],
        });
    }
    return arena_add(arena, CallExpression::create(range,
        as_ref<Expression>(callee), args.span(),
        InvocationStyleEnum::Parenthesized,
        InsideParenthesesEnum::NotInsideParentheses));
}

ASTNodeHandle ast_create_super_call(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<CallExpression::Argument> args;
    args.ensure_capacity(argument_count);
    for (size_t i = 0; i < argument_count; ++i) {
        args.unchecked_append({
            .value = as_ref<Expression>(argument_values[i]),
            .is_spread = argument_is_spread[i],
        });
    }
    return arena_add(arena, create_ast_node<SuperCall>(range, move(args)));
}

ASTNodeHandle ast_create_synthetic_constructor_super_call(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle argument_identifier)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<SuperCall>(range, SuperCall::IsPartOfSyntheticConstructor::Yes,
        CallExpression::Argument { as_ref<Expression>(argument_identifier), true }));
}

ASTNodeHandle ast_create_new_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle callee,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<CallExpression::Argument> args;
    args.ensure_capacity(argument_count);
    for (size_t i = 0; i < argument_count; ++i) {
        args.unchecked_append({
            .value = as_ref<Expression>(argument_values[i]),
            .is_spread = argument_is_spread[i],
        });
    }
    return arena_add(arena, NewExpression::create(range,
        as_ref<Expression>(callee), args.span(),
        InvocationStyleEnum::Parenthesized,
        InsideParenthesesEnum::NotInsideParentheses));
}

ASTNodeHandle ast_create_spread_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle target)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<SpreadExpression>(range, as_ref<Expression>(target)));
}

ASTNodeHandle ast_create_yield_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle argument, bool is_yield_from)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<YieldExpression>(range,
        as_nullable_ref<Expression>(argument), is_yield_from));
}

ASTNodeHandle ast_create_await_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle argument)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<AwaitExpression>(range, as_ref<Expression>(argument)));
}

ASTNodeHandle ast_create_import_call(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle specifier, ASTNodeHandle options)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ImportCall>(range,
        as_ref<Expression>(specifier), as_nullable_ref<Expression>(options)));
}

ASTNodeHandle ast_create_meta_property(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 type)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<MetaProperty>(range,
        type == 0 ? MetaProperty::Type::NewTarget : MetaProperty::Type::ImportMeta));
}

// === Statements ===

ASTNodeHandle ast_create_expression_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle expression)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ExpressionStatement>(range, as_ref<Expression>(expression)));
}

ASTNodeHandle ast_create_empty_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<EmptyStatement>(range));
}

ASTNodeHandle ast_create_return_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle argument)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ReturnStatement>(range, as_nullable_ref<Expression>(argument)));
}

ASTNodeHandle ast_create_throw_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle argument)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ThrowStatement>(range, as_ref<Expression>(argument)));
}

ASTNodeHandle ast_create_break_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* label, size_t label_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Optional<FlyString> target_label;
    if (label)
        target_label = make_fly_string(label, label_len);
    return arena_add(arena, create_ast_node<BreakStatement>(range, move(target_label)));
}

ASTNodeHandle ast_create_continue_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* label, size_t label_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Optional<FlyString> target_label;
    if (label)
        target_label = make_fly_string(label, label_len);
    return arena_add(arena, create_ast_node<ContinueStatement>(range, move(target_label)));
}

ASTNodeHandle ast_create_debugger_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<DebuggerStatement>(range));
}

ASTNodeHandle ast_create_if_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle predicate, ASTNodeHandle consequent, ASTNodeHandle alternate)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<IfStatement>(range,
        as_ref<Expression>(predicate), as_ref<Statement>(consequent), as_nullable_ref<Statement>(alternate)));
}

ASTNodeHandle ast_create_while_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle test, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<WhileStatement>(range,
        as_ref<Expression>(test), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_do_while_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle test, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<DoWhileStatement>(range,
        as_ref<Expression>(test), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle init, ASTNodeHandle test, ASTNodeHandle update, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ForStatement>(range,
        as_nullable_ref<ASTNode>(init), as_nullable_ref<Expression>(test),
        as_nullable_ref<Expression>(update), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_in_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = as_ref<ASTNode>(lhs);
    return arena_add(arena, create_ast_node<ForInStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_of_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = as_ref<ASTNode>(lhs);
    return arena_add(arena, create_ast_node<ForOfStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_await_of_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = as_ref<ASTNode>(lhs);
    return arena_add(arena, create_ast_node<ForAwaitOfStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_with_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle object, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<WithStatement>(range,
        as_ref<Expression>(object), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_labelled_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* label, size_t label_len, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto fly = make_fly_string(label, label_len);
    return arena_add(arena, create_ast_node<LabelledStatement>(range, move(fly), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_switch_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle discriminant)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<SwitchStatement>(range, as_ref<Expression>(discriminant)));
}

ASTNodeHandle ast_create_switch_case(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle test)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<SwitchCase>(range, as_nullable_ref<Expression>(test)));
}

void ast_switch_statement_add_case(ASTNodeHandle switch_stmt, ASTNodeHandle switch_case)
{
    as_node<SwitchStatement>(switch_stmt).add_case(as_ref<SwitchCase>(switch_case));
}

ASTNodeHandle ast_create_try_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle block, ASTNodeHandle handler, ASTNodeHandle finalizer)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<TryStatement>(range,
        as_ref<BlockStatement>(block),
        as_nullable_ref<CatchClause>(handler),
        as_nullable_ref<BlockStatement>(finalizer)));
}

ASTNodeHandle ast_create_catch_clause(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle parameter, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    if (!parameter)
        return arena_add(arena, create_ast_node<CatchClause>(range, as_ref<BlockStatement>(body)));
    return arena_add(arena, create_ast_node<CatchClause>(range,
        as_ref<Identifier>(parameter), as_ref<BlockStatement>(body)));
}

// === Declarations ===

ASTNodeHandle ast_create_variable_declaration(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 kind,
    ASTNodeHandle const* declarators, size_t declarator_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<VariableDeclarator const>> decls;
    decls.ensure_capacity(declarator_count);
    for (size_t i = 0; i < declarator_count; ++i)
        decls.unchecked_append(as_ref<VariableDeclarator>(declarators[i]));

    DeclarationKind dk;
    switch (kind) {
    case 0:
        dk = DeclarationKind::Var;
        break;
    case 1:
        dk = DeclarationKind::Let;
        break;
    case 2:
        dk = DeclarationKind::Const;
        break;
    default:
        dk = DeclarationKind::Var;
        break;
    }

    return arena_add(arena, create_ast_node<VariableDeclaration>(range, dk, move(decls)));
}

ASTNodeHandle ast_create_variable_declarator(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle target, ASTNodeHandle init)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<BindingPattern const>> target_variant = as_ref<Identifier>(target);
    return arena_add(arena, create_ast_node<VariableDeclarator>(range,
        move(target_variant), as_nullable_ref<Expression>(init)));
}

ASTNodeHandle ast_create_using_declaration(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* declarators, size_t declarator_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<VariableDeclarator const>> decls;
    decls.ensure_capacity(declarator_count);
    for (size_t i = 0; i < declarator_count; ++i)
        decls.unchecked_append(as_ref<VariableDeclarator>(declarators[i]));
    return arena_add(arena, create_ast_node<UsingDeclaration>(range, move(decls)));
}

// === Object/Array expressions ===

ASTNodeHandle ast_create_object_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* properties, size_t property_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<ObjectProperty>> props;
    props.ensure_capacity(property_count);
    for (size_t i = 0; i < property_count; ++i)
        props.unchecked_append(as_mut_ref<ObjectProperty>(properties[i]));
    return arena_add(arena, create_ast_node<ObjectExpression>(range, move(props)));
}

ASTNodeHandle ast_create_object_property(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle key, ASTNodeHandle value, u8 type, bool is_method)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ObjectProperty>(range,
        as_ref<Expression>(key), as_nullable_ref<Expression>(value),
        static_cast<ObjectProperty::Type>(type), is_method));
}

ASTNodeHandle ast_create_array_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* elements, size_t element_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<RefPtr<Expression const>> elems;
    elems.ensure_capacity(element_count);
    for (size_t i = 0; i < element_count; ++i)
        elems.unchecked_append(as_nullable_ref<Expression>(elements[i]));
    return arena_add(arena, create_ast_node<ArrayExpression>(range, move(elems)));
}

// === Template literals ===

ASTNodeHandle ast_create_template_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* expressions, size_t expression_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<Expression const>> exprs;
    exprs.ensure_capacity(expression_count);
    for (size_t i = 0; i < expression_count; ++i)
        exprs.unchecked_append(as_ref<Expression>(expressions[i]));
    return arena_add(arena, create_ast_node<TemplateLiteral>(range, move(exprs)));
}

ASTNodeHandle ast_create_template_literal_with_raw_strings(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle const* expressions, size_t expression_count,
    ASTNodeHandle const* raw_strings, size_t raw_string_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Vector<NonnullRefPtr<Expression const>> exprs;
    exprs.ensure_capacity(expression_count);
    for (size_t i = 0; i < expression_count; ++i)
        exprs.unchecked_append(as_ref<Expression>(expressions[i]));
    Vector<NonnullRefPtr<StringLiteral const>> raws;
    raws.ensure_capacity(raw_string_count);
    for (size_t i = 0; i < raw_string_count; ++i)
        raws.unchecked_append(static_cast<StringLiteral const&>(*static_cast<ASTNode*>(raw_strings[i])));
    return arena_add(arena, create_ast_node<TemplateLiteral>(range, move(exprs), move(raws)));
}

ASTNodeHandle ast_create_tagged_template_literal(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle tag, ASTNodeHandle template_literal)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<TaggedTemplateLiteral>(range,
        as_ref<Expression>(tag), as_ref<TemplateLiteral>(template_literal)));
}

// === Functions ===

ASTNodeHandle ast_create_function_parameters_empty()
{
    return const_cast<FunctionParameters*>(FunctionParameters::empty().ptr());
}

ASTNodeHandle ast_create_function_parameters(ASTArenaHandle arena_handle,
    ASTNodeHandle const* bindings, ASTNodeHandle const* default_values,
    bool const* is_rest_flags, bool const* is_pattern_flags, size_t count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    Vector<FunctionParameter> params;
    params.ensure_capacity(count);
    for (size_t i = 0; i < count; ++i) {
        RefPtr<Expression const> default_value;
        if (default_values[i])
            default_value = as_ref<Expression>(default_values[i]);
        if (is_pattern_flags && is_pattern_flags[i]) {
            params.empend(
                Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<BindingPattern const>> { NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(bindings[i])) },
                move(default_value),
                is_rest_flags[i]);
        } else {
            params.empend(
                Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<BindingPattern const>> { as_ref<Identifier>(bindings[i]) },
                move(default_value),
                is_rest_flags[i]);
        }
    }
    auto parameters = FunctionParameters::create(move(params));
    arena.parameters.append(parameters);
    return const_cast<FunctionParameters*>(parameters.ptr());
}

ASTNodeHandle ast_create_function_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle name,
    u32 source_text_start, u32 source_text_len,
    ASTNodeHandle body, ASTNodeHandle parameters,
    i32 function_length, u8 kind,
    bool is_strict_mode, bool is_arrow_function,
    bool uses_this, bool uses_this_from_environment,
    bool contains_direct_call_to_eval, bool might_need_arguments_object)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto& code = *static_cast<SourceCode const*>(source_code);
    auto source_text = code.code_view().substring_view(source_text_start, source_text_len);

    FunctionParsingInsights insights;
    insights.uses_this = uses_this;
    insights.uses_this_from_environment = uses_this_from_environment;
    insights.contains_direct_call_to_eval = contains_direct_call_to_eval;
    insights.might_need_arguments_object = might_need_arguments_object;

    auto& params = *static_cast<FunctionParameters const*>(parameters);
    return arena_add(arena, create_ast_node<FunctionExpression>(range,
        name ? as_ref<Identifier>(name) : RefPtr<Identifier const> {},
        source_text,
        as_ref<Statement>(body),
        NonnullRefPtr<FunctionParameters const>(params),
        function_length,
        static_cast<FunctionKind>(kind),
        is_strict_mode,
        insights,
        is_arrow_function));
}

ASTNodeHandle ast_create_function_declaration(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle name,
    u32 source_text_start, u32 source_text_len,
    ASTNodeHandle body, ASTNodeHandle parameters,
    i32 function_length, u8 kind,
    bool is_strict_mode,
    bool uses_this, bool uses_this_from_environment,
    bool contains_direct_call_to_eval, bool might_need_arguments_object)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto& code = *static_cast<SourceCode const*>(source_code);
    auto source_text = code.code_view().substring_view(source_text_start, source_text_len);

    FunctionParsingInsights insights;
    insights.uses_this = uses_this;
    insights.uses_this_from_environment = uses_this_from_environment;
    insights.contains_direct_call_to_eval = contains_direct_call_to_eval;
    insights.might_need_arguments_object = might_need_arguments_object;

    auto& params = *static_cast<FunctionParameters const*>(parameters);
    return arena_add(arena, create_ast_node<FunctionDeclaration>(range,
        name ? as_ref<Identifier>(name) : RefPtr<Identifier const> {},
        source_text,
        as_ref<Statement>(body),
        NonnullRefPtr<FunctionParameters const>(params),
        function_length,
        static_cast<FunctionKind>(kind),
        is_strict_mode,
        insights));
}

// === Classes ===

ASTNodeHandle ast_create_class_expression(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle name,
    u32 source_text_start, u32 source_text_len,
    ASTNodeHandle constructor,
    ASTNodeHandle super_class,
    ASTNodeHandle const* elements, size_t element_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto& code = *static_cast<SourceCode const*>(source_code);
    auto source_text = code.code_view().substring_view(source_text_start, source_text_len);

    Vector<NonnullRefPtr<ClassElement const>> elems;
    elems.ensure_capacity(element_count);
    for (size_t i = 0; i < element_count; ++i)
        elems.unchecked_append(as_ref<ClassElement>(elements[i]));

    return arena_add(arena, create_ast_node<ClassExpression>(range,
        name ? as_ref<Identifier>(name) : RefPtr<Identifier const> {},
        source_text,
        constructor ? as_ref<FunctionExpression>(constructor) : RefPtr<FunctionExpression const> {},
        super_class ? as_ref<Expression>(super_class) : RefPtr<Expression const> {},
        move(elems)));
}

ASTNodeHandle ast_create_class_declaration(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle class_expression)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ClassDeclaration>(range, as_ref<ClassExpression>(class_expression)));
}

ASTNodeHandle ast_create_class_method(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle key, ASTNodeHandle function, u8 kind, bool is_static)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ClassMethod>(range,
        as_ref<Expression>(key), as_ref<FunctionExpression>(function),
        static_cast<ClassMethod::Kind>(kind), is_static));
}

ASTNodeHandle ast_create_class_field(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle key, ASTNodeHandle init, bool is_static)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<ClassField>(range,
        as_ref<Expression>(key), as_nullable_ref<Expression>(init), is_static));
}

ASTNodeHandle ast_create_static_initializer(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle function_body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<StaticInitializer>(range, as_mut_ref<FunctionBody>(function_body)));
}

// === Scope declarations ===

void ast_scope_node_add_var_scoped_declaration(ASTNodeHandle scope_node, ASTNodeHandle declaration)
{
    as_node<ScopeNode>(scope_node).add_var_scoped_declaration(as_ref<Declaration>(declaration));
}

void ast_scope_node_add_lexical_declaration(ASTNodeHandle scope_node, ASTNodeHandle declaration)
{
    as_node<ScopeNode>(scope_node).add_lexical_declaration(as_ref<Declaration>(declaration));
}

void ast_scope_node_add_hoisted_function(ASTNodeHandle scope_node, ASTNodeHandle function_declaration)
{
    as_node<ScopeNode>(scope_node).add_hoisted_function(as_ref<FunctionDeclaration>(function_declaration));
}

void ast_scope_node_shrink_to_fit(ASTNodeHandle scope_node)
{
    as_node<ScopeNode>(scope_node).shrink_to_fit();
}

void ast_scope_build_function_scope_data(
    ASTNodeHandle scope_node_handle,
    uint16_t const* var_names_data,
    uint32_t const* var_name_offsets,
    uint32_t const* var_name_lengths,
    ASTNodeHandle const* var_identifiers,
    uint8_t const* var_is_parameter,
    size_t var_count,
    uint8_t has_argument_parameter)
{
    auto& scope_node = as_node<ScopeNode>(scope_node_handle);

    auto data = make<FunctionScopeData>();

    // Extract functions_to_initialize from var-scoped function declarations (in reverse order, deduplicated).
    HashTable<Utf16FlyString> seen_function_names;
    for (ssize_t i = scope_node.var_declaration_count() - 1; i >= 0; i--) {
        auto const& declaration = scope_node.var_declarations()[i];
        if (is<FunctionDeclaration>(declaration)) {
            auto& function_decl = static_cast<FunctionDeclaration const&>(*declaration);
            if (seen_function_names.set(function_decl.name()) == AK::HashSetResult::InsertedNewEntry)
                data->functions_to_initialize.append(static_ptr_cast<FunctionDeclaration const>(declaration));
        }
    }

    data->has_function_named_arguments = seen_function_names.contains("arguments"_utf16_fly_string);
    data->has_argument_parameter = has_argument_parameter != 0;

    // Check if "arguments" is lexically declared.
    MUST(scope_node.for_each_lexically_declared_identifier([&](auto const& identifier) {
        if (identifier.string() == "arguments"_utf16_fly_string)
            data->has_lexically_declared_arguments = true;
    }));

    // Build vars_to_initialize from Rust scope variables.
    HashTable<Utf16FlyString> seen_var_names;
    for (size_t i = 0; i < var_count; i++) {
        auto name = Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(var_names_data + var_name_offsets[i]), var_name_lengths[i]));
        auto& identifier = as_node<Identifier>(var_identifiers[i]);
        bool is_parameter = var_is_parameter[i] != 0;
        bool is_non_local = !identifier.is_local();

        if (seen_var_names.set(name) == AK::HashSetResult::InsertedNewEntry) {
            data->vars_to_initialize.append({
                .identifier = identifier,
                .is_parameter = is_parameter,
                .is_function_name = seen_function_names.contains(name),
            });

            data->var_names.set(name);

            if (is_non_local) {
                data->non_local_var_count_for_parameter_expressions++;
                if (!is_parameter)
                    data->non_local_var_count++;
            }
        }
    }

    scope_node.set_function_scope_data(move(data));
}

// === SwitchCase ===

void ast_switch_case_append(ASTNodeHandle switch_case, ASTNodeHandle statement)
{
    as_node<SwitchCase>(switch_case).append(as_ref<Statement>(statement));
}

// === BindingPattern ===

ASTNodeHandle ast_create_binding_pattern(ASTArenaHandle arena_handle, u8 kind)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto pattern = adopt_ref(*new BindingPattern);
    pattern->kind = kind == 0 ? BindingPattern::Kind::Array : BindingPattern::Kind::Object;
    auto* raw = pattern.ptr();
    arena.binding_patterns.append(move(pattern));
    return static_cast<ASTNodeHandle>(raw);
}

void ast_binding_pattern_append_entry(
    ASTNodeHandle pattern_handle,
    ASTNodeHandle name, u8 name_type,
    ASTNodeHandle alias, u8 alias_type,
    ASTNodeHandle initializer, bool is_rest)
{
    auto& pattern = *static_cast<BindingPattern*>(pattern_handle);

    Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<Expression const>, Empty> name_variant;
    switch (name_type) {
    case 1:
        name_variant = as_ref<Identifier>(name);
        break;
    case 2:
        name_variant = as_ref<Expression>(name);
        break;
    default:
        name_variant = Empty {};
        break;
    }

    Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<BindingPattern const>, NonnullRefPtr<MemberExpression const>, Empty> alias_variant;
    switch (alias_type) {
    case 1:
        alias_variant = as_ref<Identifier>(alias);
        break;
    case 2:
        alias_variant = NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(alias));
        break;
    case 3:
        alias_variant = as_ref<MemberExpression>(alias);
        break;
    default:
        alias_variant = Empty {};
        break;
    }

    RefPtr<Expression const> init;
    if (initializer)
        init = as_ref<Expression>(initializer);

    pattern.entries.append(BindingPattern::BindingEntry { move(name_variant), move(alias_variant), move(init), is_rest });
}

ASTNodeHandle ast_create_variable_declarator_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle pattern, ASTNodeHandle init)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<Identifier const>, NonnullRefPtr<BindingPattern const>> target_variant = NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern));
    return arena_add(arena, create_ast_node<VariableDeclarator>(range,
        move(target_variant), as_nullable_ref<Expression>(init)));
}

ASTNodeHandle ast_create_catch_clause_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle pattern, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<CatchClause>(range,
        NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern)),
        as_ref<BlockStatement>(body)));
}

ASTNodeHandle ast_create_for_in_statement_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern));
    return arena_add(arena, create_ast_node<ForInStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_of_statement_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern));
    return arena_add(arena, create_ast_node<ForOfStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_for_await_of_statement_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    Variant<NonnullRefPtr<ASTNode const>, NonnullRefPtr<BindingPattern const>> lhs_variant = NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern));
    return arena_add(arena, create_ast_node<ForAwaitOfStatement>(range,
        move(lhs_variant), as_ref<Expression>(rhs), as_ref<Statement>(body)));
}

ASTNodeHandle ast_create_assignment_expression_with_pattern(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u8 op, ASTNodeHandle pattern, ASTNodeHandle rhs)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    return arena_add(arena, create_ast_node<AssignmentExpression>(range,
        static_cast<AssignmentOp>(op), NonnullRefPtr<BindingPattern const>(*static_cast<BindingPattern const*>(pattern)), as_ref<Expression>(rhs)));
}

// === OptionalChain ===

struct OptionalChainBuilder {
    Vector<OptionalChain::Reference> references;
};

OptionalChainBuilderHandle ast_create_optional_chain_builder()
{
    return new OptionalChainBuilder();
}

void ast_optional_chain_builder_append_member(OptionalChainBuilderHandle builder_handle,
    ASTNodeHandle identifier, bool is_optional)
{
    auto& builder = *static_cast<OptionalChainBuilder*>(builder_handle);
    builder.references.append(OptionalChain::MemberReference {
        as_ref<Identifier>(identifier),
        is_optional ? OptionalChain::Mode::Optional : OptionalChain::Mode::NotOptional,
    });
}

void ast_optional_chain_builder_append_computed(OptionalChainBuilderHandle builder_handle,
    ASTNodeHandle expression, bool is_optional)
{
    auto& builder = *static_cast<OptionalChainBuilder*>(builder_handle);
    builder.references.append(OptionalChain::ComputedReference {
        as_ref<Expression>(expression),
        is_optional ? OptionalChain::Mode::Optional : OptionalChain::Mode::NotOptional,
    });
}

void ast_optional_chain_builder_append_call(OptionalChainBuilderHandle builder_handle,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread,
    size_t argument_count, bool is_optional)
{
    auto& builder = *static_cast<OptionalChainBuilder*>(builder_handle);
    Vector<CallExpression::Argument> args;
    args.ensure_capacity(argument_count);
    for (size_t i = 0; i < argument_count; ++i) {
        args.unchecked_append({
            .value = as_ref<Expression>(argument_values[i]),
            .is_spread = argument_is_spread[i],
        });
    }
    builder.references.append(OptionalChain::Call {
        move(args),
        is_optional ? OptionalChain::Mode::Optional : OptionalChain::Mode::NotOptional,
    });
}

void ast_optional_chain_builder_append_private_member(OptionalChainBuilderHandle builder_handle,
    ASTNodeHandle private_identifier, bool is_optional)
{
    auto& builder = *static_cast<OptionalChainBuilder*>(builder_handle);
    builder.references.append(OptionalChain::PrivateMemberReference {
        as_ref<PrivateIdentifier>(private_identifier),
        is_optional ? OptionalChain::Mode::Optional : OptionalChain::Mode::NotOptional,
    });
}

ASTNodeHandle ast_create_optional_chain(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle base, OptionalChainBuilderHandle builder_handle)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto& builder = *static_cast<OptionalChainBuilder*>(builder_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);
    auto result = arena_add(arena, create_ast_node<OptionalChain>(range,
        as_ref<Expression>(base), move(builder.references)));
    delete &builder;
    return result;
}

bool ast_is_identifier(ASTNodeHandle handle)
{
    return is<Identifier>(*static_cast<ASTNode const*>(handle));
}

bool ast_is_member_expression(ASTNodeHandle handle)
{
    return is<MemberExpression>(*static_cast<ASTNode const*>(handle));
}

bool ast_is_object_expression(ASTNodeHandle handle)
{
    return is<ObjectExpression>(*static_cast<ASTNode const*>(handle));
}

bool ast_is_array_expression(ASTNodeHandle handle)
{
    return is<ArrayExpression>(*static_cast<ASTNode const*>(handle));
}

bool ast_is_call_expression(ASTNodeHandle handle)
{
    return is<CallExpression>(*static_cast<ASTNode const*>(handle));
}

ASTNodeHandle ast_create_import_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    u16 const* module_specifier, size_t module_specifier_len,
    FFIImportEntry const* entries, size_t entry_count)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);

    auto specifier_view = make_utf16_view(module_specifier, module_specifier_len);
    ModuleRequest module_request { Utf16FlyString::from_utf16(specifier_view) };

    Vector<ImportEntry> import_entries;
    import_entries.ensure_capacity(entry_count);
    for (size_t i = 0; i < entry_count; ++i) {
        auto const& e = entries[i];
        auto local_name = Utf16FlyString::from_utf16(make_utf16_view(e.local_name, e.local_name_len));
        if (e.import_name_len == SIZE_MAX) {
            // Namespace import (no import_name).
            import_entries.unchecked_append(ImportEntry({}, move(local_name)));
        } else {
            auto import_name = Utf16FlyString::from_utf16(make_utf16_view(e.import_name, e.import_name_len));
            import_entries.unchecked_append(ImportEntry(move(import_name), move(local_name)));
        }
    }

    return arena_add(arena, create_ast_node<ImportStatement>(range, move(module_request), move(import_entries)));
}

ASTNodeHandle ast_create_export_statement(ASTArenaHandle arena_handle, SourceCodeHandle source_code,
    u32 start_line, u32 start_column, u32 start_offset,
    u32 end_line, u32 end_column, u32 end_offset,
    ASTNodeHandle statement_or_null,
    FFIExportEntry const* entries, size_t entry_count,
    bool is_default,
    u16 const* from_specifier, size_t from_specifier_len)
{
    auto& arena = *static_cast<ASTArena*>(arena_handle);
    auto range = make_range(source_code, start_line, start_column, start_offset, end_line, end_column, end_offset);

    Vector<ExportEntry> export_entries;
    export_entries.ensure_capacity(entry_count);
    for (size_t i = 0; i < entry_count; ++i) {
        auto const& e = entries[i];
        Optional<Utf16FlyString> export_name;
        if (e.export_name_len != SIZE_MAX)
            export_name = Utf16FlyString::from_utf16(make_utf16_view(e.export_name, e.export_name_len));
        Optional<Utf16FlyString> local_or_import_name;
        if (e.local_or_import_name_len != SIZE_MAX)
            local_or_import_name = Utf16FlyString::from_utf16(make_utf16_view(e.local_or_import_name, e.local_or_import_name_len));

        ExportEntry::Kind kind;
        switch (e.kind) {
        case 0:
            kind = ExportEntry::Kind::NamedExport;
            break;
        case 1:
            kind = ExportEntry::Kind::ModuleRequestAll;
            break;
        case 2:
            kind = ExportEntry::Kind::ModuleRequestAllButDefault;
            break;
        case 3:
            kind = ExportEntry::Kind::EmptyNamedExport;
            break;
        default:
            VERIFY_NOT_REACHED();
        }
        export_entries.unchecked_append(ExportEntry(kind, move(export_name), move(local_or_import_name)));
    }

    RefPtr<ASTNode const> statement;
    if (statement_or_null)
        statement = as_ref<ASTNode>(statement_or_null);

    Optional<ModuleRequest> module_request;
    if (from_specifier_len != SIZE_MAX) {
        auto specifier_view = make_utf16_view(from_specifier, from_specifier_len);
        module_request = ModuleRequest { Utf16FlyString::from_utf16(specifier_view) };
    }

    return arena_add(arena, create_ast_node<ExportStatement>(range, move(statement), move(export_entries), is_default, move(module_request)));
}

void ast_import_statement_add_attribute(ASTNodeHandle import_stmt,
    u16 const* key, size_t key_len,
    u16 const* value, size_t value_len)
{
    auto& stmt = const_cast<ImportStatement&>(static_cast<ImportStatement const&>(*static_cast<ASTNode*>(import_stmt)));
    auto key_view = make_utf16_view(key, key_len);
    auto value_view = make_utf16_view(value, value_len);
    const_cast<ModuleRequest&>(stmt.module_request()).add_attribute(
        Utf16String::from_utf16(key_view), Utf16String::from_utf16(value_view));
}

void ast_export_statement_add_attribute(ASTNodeHandle export_stmt,
    u16 const* key, size_t key_len,
    u16 const* value, size_t value_len)
{
    auto& stmt = const_cast<ExportStatement&>(static_cast<ExportStatement const&>(*static_cast<ASTNode*>(export_stmt)));
    auto key_view = make_utf16_view(key, key_len);
    auto value_view = make_utf16_view(value, value_len);
    const_cast<ModuleRequest&>(stmt.module_request()).add_attribute(
        Utf16String::from_utf16(key_view), Utf16String::from_utf16(value_view));
}

void ast_program_append_import(ASTNodeHandle program, ASTNodeHandle import_stmt)
{
    auto& prog = static_cast<Program&>(*static_cast<ASTNode*>(program));
    prog.append_import(as_ref<ImportStatement>(import_stmt));
}

void ast_program_append_export(ASTNodeHandle program, ASTNodeHandle export_stmt)
{
    auto& prog = static_cast<Program&>(*static_cast<ASTNode*>(program));
    prog.append_export(as_ref<ExportStatement>(export_stmt));
}

void ast_program_set_has_top_level_await(ASTNodeHandle program)
{
    static_cast<Program&>(*static_cast<ASTNode*>(program)).set_has_top_level_await();
}

// Buffers for converting Utf16FlyString names to u16 slices for FFI.
// These are used by the export name extraction functions.
static thread_local Vector<Vector<u16>> s_name_buffers;

static FFIUtf16Slice fly_string_to_buffered_slice(Utf16FlyString const& s)
{
    auto view = s.view();
    auto len = view.length_in_code_units();
    Vector<u16> buf;
    buf.ensure_capacity(len);
    if (view.has_ascii_storage()) {
        auto ascii = view.ascii_span();
        for (auto ch : ascii)
            buf.unchecked_append(static_cast<u16>(ch));
    } else {
        auto span = view.utf16_span();
        for (size_t i = 0; i < span.size(); ++i)
            buf.unchecked_append(static_cast<u16>(span[i]));
    }
    s_name_buffers.append(move(buf));
    auto& stored = s_name_buffers.last();
    return { stored.data(), stored.size() };
}

size_t ast_get_declaration_export_names(ASTNodeHandle declaration,
    FFIUtf16Slice* out_names, size_t max_names)
{
    auto& node = *static_cast<ASTNode*>(declaration);
    size_t count = 0;
    s_name_buffers.clear_with_capacity();

    if (is<FunctionDeclaration>(node)) {
        auto& func = static_cast<FunctionDeclaration const&>(node);
        if (count < max_names)
            out_names[count] = fly_string_to_buffered_slice(func.name());
        count++;
    } else if (is<ClassDeclaration>(node)) {
        auto& cls = static_cast<ClassDeclaration const&>(node);
        if (count < max_names)
            out_names[count] = fly_string_to_buffered_slice(cls.name());
        count++;
    } else if (is<VariableDeclaration>(node)) {
        auto& vars = static_cast<VariableDeclaration const&>(node);
        for (auto& decl : vars.declarations()) {
            decl->target().visit(
                [&](NonnullRefPtr<Identifier const> const& identifier) {
                    if (count < max_names)
                        out_names[count] = fly_string_to_buffered_slice(identifier->string());
                    count++;
                },
                [&](NonnullRefPtr<BindingPattern const> const& binding) {
                    MUST(binding->for_each_bound_identifier([&](auto& identifier) {
                        if (count < max_names)
                            out_names[count] = fly_string_to_buffered_slice(identifier.string());
                        count++;
                    }));
                });
        }
    }
    return count;
}

FFIUtf16Slice ast_get_function_name(ASTNodeHandle function_decl)
{
    s_name_buffers.clear_with_capacity();
    auto& node = *static_cast<ASTNode*>(function_decl);
    if (is<FunctionDeclaration>(node))
        return fly_string_to_buffered_slice(static_cast<FunctionDeclaration const&>(node).name());
    if (is<FunctionExpression>(node))
        return fly_string_to_buffered_slice(static_cast<FunctionExpression const&>(node).name());
    return { nullptr, 0 };
}

FFIUtf16Slice ast_get_class_name(ASTNodeHandle class_decl)
{
    s_name_buffers.clear_with_capacity();
    auto& node = *static_cast<ASTNode*>(class_decl);
    if (is<ClassDeclaration>(node))
        return fly_string_to_buffered_slice(static_cast<ClassDeclaration const&>(node).name());
    if (is<ClassExpression>(node))
        return fly_string_to_buffered_slice(static_cast<ClassExpression const&>(node).name());
    return { nullptr, 0 };
}

bool ast_function_has_name(ASTNodeHandle function_decl)
{
    auto& node = *static_cast<ASTNode*>(function_decl);
    if (is<FunctionDeclaration>(node))
        return !static_cast<FunctionDeclaration const&>(node).name().is_empty();
    return false;
}

} // extern "C"

// === High-level integration ===

// Declared in Rust (lib.rs)
extern "C" ASTNodeHandle rust_parse_program(
    u16 const* source,
    size_t source_len,
    void const* source_code,
    u8 program_type,
    bool starts_in_strict_mode,
    bool initiated_by_eval,
    bool in_eval_function_context,
    bool allow_super_property_lookup,
    bool allow_super_constructor_call,
    bool in_class_field_initializer,
    bool* out_has_errors);

namespace JS {

NonnullRefPtr<Program> rust_parse(
    NonnullRefPtr<SourceCode const> source_code,
    Program::Type program_type,
    bool starts_in_strict_mode,
    bool initiated_by_eval,
    bool in_eval_function_context,
    bool allow_super_property_lookup,
    bool allow_super_constructor_call,
    bool in_class_field_initializer,
    bool& out_has_errors)
{
    auto const& code_view = source_code->code_view();
    auto length = code_view.length_in_code_units();

    ASTNodeHandle program;
    out_has_errors = false;

    u8 pt = program_type == Program::Type::Script ? 0 : 1;

    if (code_view.has_ascii_storage()) {
        // Widen ASCII to UTF-16
        auto ascii = code_view.ascii_span();
        Vector<u16> utf16_buf;
        utf16_buf.ensure_capacity(length);
        for (size_t i = 0; i < length; ++i)
            utf16_buf.unchecked_append(static_cast<u16>(ascii[i]));
        program = rust_parse_program(utf16_buf.data(), length, source_code.ptr(), pt, starts_in_strict_mode,
            initiated_by_eval, in_eval_function_context, allow_super_property_lookup,
            allow_super_constructor_call, in_class_field_initializer, &out_has_errors);
    } else {
        auto utf16 = code_view.utf16_span();
        program = rust_parse_program(reinterpret_cast<u16 const*>(utf16.data()), length, source_code.ptr(), pt, starts_in_strict_mode,
            initiated_by_eval, in_eval_function_context, allow_super_property_lookup,
            allow_super_constructor_call, in_class_field_initializer, &out_has_errors);
    }

    if (out_has_errors || !program) {
        out_has_errors = true;
        return adopt_ref(*new Program({ source_code, {}, {} }, program_type));
    }

    // The Rust side added an extra ref before dropping the arena.
    // Adopt it without incrementing the refcount again.
    return adopt_ref(static_cast<Program&>(*static_cast<ASTNode*>(program)));
}

} // namespace JS
