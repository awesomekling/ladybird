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
#include <LibJS/SourceCode.h>
#include <LibJS/SourceRange.h>

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
        Vector<LocalVariable> {},
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
        insights,
        Vector<LocalVariable> {}));
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

} // extern "C"

// === High-level integration ===

// Declared in Rust (lib.rs)
extern "C" ASTNodeHandle rust_parse_program(
    u16 const* source,
    size_t source_len,
    void const* source_code,
    u8 program_type,
    bool starts_in_strict_mode,
    bool* out_has_errors);

namespace JS {

NonnullRefPtr<Program> rust_parse(NonnullRefPtr<SourceCode const> source_code, Program::Type program_type, bool starts_in_strict_mode, bool& out_has_errors)
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
        program = rust_parse_program(utf16_buf.data(), length, source_code.ptr(), pt, starts_in_strict_mode, &out_has_errors);
    } else {
        auto utf16 = code_view.utf16_span();
        program = rust_parse_program(reinterpret_cast<u16 const*>(utf16.data()), length, source_code.ptr(), pt, starts_in_strict_mode, &out_has_errors);
    }

    // The Rust side added an extra ref before dropping the arena.
    // Adopt it without incrementing the refcount again.
    return adopt_ref(static_cast<Program&>(*static_cast<ASTNode*>(program)));
}

} // namespace JS
