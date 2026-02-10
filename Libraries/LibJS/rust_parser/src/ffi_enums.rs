/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Numeric constants that must match C++ enum definitions or FFI protocols.
//!
//! The Rust parser creates C++ AST nodes via FFI, passing enum values as
//! `u8`. These constants define the mapping. If a C++ enum is reordered,
//! these must be updated to match.
//!
//! Each group references the C++ header file and enum it mirrors.
//! Groups marked "FFI protocol" are ad-hoc discriminants used by
//! ASTFactory.cpp to pick the right Variant alternative.

// --- AST.h: enum class BinaryOp ---

#[allow(dead_code, non_snake_case)]
pub mod BinaryOp {
    pub const ADDITION: u8 = 0;
    pub const SUBTRACTION: u8 = 1;
    pub const MULTIPLICATION: u8 = 2;
    pub const DIVISION: u8 = 3;
    pub const MODULO: u8 = 4;
    pub const EXPONENTIATION: u8 = 5;
    pub const STRICTLY_EQUALS: u8 = 6;
    pub const STRICTLY_INEQUALS: u8 = 7;
    pub const LOOSELY_EQUALS: u8 = 8;
    pub const LOOSELY_INEQUALS: u8 = 9;
    pub const GREATER_THAN: u8 = 10;
    pub const GREATER_THAN_EQUALS: u8 = 11;
    pub const LESS_THAN: u8 = 12;
    pub const LESS_THAN_EQUALS: u8 = 13;
    pub const BITWISE_AND: u8 = 14;
    pub const BITWISE_OR: u8 = 15;
    pub const BITWISE_XOR: u8 = 16;
    pub const LEFT_SHIFT: u8 = 17;
    pub const RIGHT_SHIFT: u8 = 18;
    pub const UNSIGNED_RIGHT_SHIFT: u8 = 19;
    pub const IN: u8 = 20;
    pub const INSTANCE_OF: u8 = 21;
}

// --- AST.h: enum class LogicalOp ---

#[allow(non_snake_case)]
pub mod LogicalOp {
    pub const AND: u8 = 0;
    pub const OR: u8 = 1;
    pub const NULLISH_COALESCING: u8 = 2;
}

// --- AST.h: enum class UnaryOp ---

#[allow(non_snake_case)]
pub mod UnaryOp {
    pub const BITWISE_NOT: u8 = 0;
    pub const NOT: u8 = 1;
    pub const PLUS: u8 = 2;
    pub const MINUS: u8 = 3;
    pub const TYPEOF: u8 = 4;
    pub const VOID: u8 = 5;
    pub const DELETE: u8 = 6;
}

// --- AST.h: enum class UpdateOp ---

#[allow(non_snake_case)]
pub mod UpdateOp {
    pub const INCREMENT: u8 = 0;
    pub const DECREMENT: u8 = 1;
}

// --- AST.h: enum class AssignmentOp ---

#[allow(dead_code, non_snake_case)]
pub mod AssignmentOp {
    pub const ASSIGNMENT: u8 = 0;
    pub const ADDITION_ASSIGNMENT: u8 = 1;
    pub const SUBTRACTION_ASSIGNMENT: u8 = 2;
    pub const MULTIPLICATION_ASSIGNMENT: u8 = 3;
    pub const DIVISION_ASSIGNMENT: u8 = 4;
    pub const MODULO_ASSIGNMENT: u8 = 5;
    pub const EXPONENTIATION_ASSIGNMENT: u8 = 6;
    pub const BITWISE_AND_ASSIGNMENT: u8 = 7;
    pub const BITWISE_OR_ASSIGNMENT: u8 = 8;
    pub const BITWISE_XOR_ASSIGNMENT: u8 = 9;
    pub const LEFT_SHIFT_ASSIGNMENT: u8 = 10;
    pub const RIGHT_SHIFT_ASSIGNMENT: u8 = 11;
    pub const UNSIGNED_RIGHT_SHIFT_ASSIGNMENT: u8 = 12;
    pub const AND_ASSIGNMENT: u8 = 13;
    pub const OR_ASSIGNMENT: u8 = 14;
    pub const NULLISH_ASSIGNMENT: u8 = 15;
}

// --- AST.h: enum class MetaProperty::Type ---

#[allow(non_snake_case)]
pub mod MetaProperty {
    pub const NEW_TARGET: u8 = 0;
    pub const IMPORT_META: u8 = 1;
}

// --- AST.h: ObjectProperty::Type ---

#[allow(non_snake_case)]
pub mod ObjectProperty {
    pub const KEY_VALUE: u8 = 0;
    pub const GETTER: u8 = 1;
    pub const SETTER: u8 = 2;
    pub const SPREAD: u8 = 3;
    pub const PROTO_SETTER: u8 = 4;
}

// --- AST.h: ClassMethod::Kind ---

#[allow(non_snake_case)]
pub mod ClassMethod {
    pub const METHOD: u8 = 0;
    pub const GETTER: u8 = 1;
    pub const SETTER: u8 = 2;
}

// --- AST.h: BindingPattern::Kind ---

#[allow(non_snake_case)]
pub mod BindingPattern {
    pub const ARRAY: u8 = 0;
    pub const OBJECT: u8 = 1;
}

// --- ASTFactory.cpp: BindingEntry name discriminant (FFI protocol) ---
// Selects which Variant alternative to use for BindingPattern::BindingEntry::name.
// Variant<NonnullRefPtr<Identifier>, NonnullRefPtr<Expression>, Empty>

#[allow(non_snake_case)]
pub mod BindingEntryName {
    pub const EMPTY: u8 = 0;
    pub const IDENTIFIER: u8 = 1;
    pub const EXPRESSION: u8 = 2;
}

// --- ASTFactory.cpp: BindingEntry alias discriminant (FFI protocol) ---
// Selects which Variant alternative to use for BindingPattern::BindingEntry::alias.
// Variant<NonnullRefPtr<Identifier>, NonnullRefPtr<BindingPattern>, NonnullRefPtr<MemberExpression>, Empty>

#[allow(non_snake_case)]
pub mod BindingEntryAlias {
    pub const EMPTY: u8 = 0;
    pub const IDENTIFIER: u8 = 1;
    pub const BINDING_PATTERN: u8 = 2;
    pub const MEMBER_EXPRESSION: u8 = 3;
}

// --- AST.h: enum class DeclarationKind ---
// Note: the C++ enum has a leading None=0, so Var starts at 1.

#[allow(non_snake_case)]
pub mod DeclarationKind {
    pub const VAR: u8 = 1;
    pub const LET: u8 = 2;
    pub const CONST: u8 = 3;
}

// --- LocalVariable.h: LocalVariable::DeclarationKind ---

#[allow(non_snake_case)]
pub mod LocalVariable {
    pub const VAR: u8 = 0;
    pub const LET_OR_CONST: u8 = 1;
    pub const FUNCTION: u8 = 2;
    pub const ARGUMENTS_OBJECT: u8 = 3;
    pub const CATCH_CLAUSE_PARAMETER: u8 = 4;
}
