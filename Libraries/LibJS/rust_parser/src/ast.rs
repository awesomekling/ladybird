/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Rust AST types for JavaScript.
//!
//! This module defines the Abstract Syntax Tree using idiomatic Rust enums
//! instead of the C++ class hierarchy. Every node carries a `SourceRange`
//! for error messages and source maps.
//!
//! ## Design
//!
//! - `Expression` and `Statement` are flat enums — pattern matching replaces
//!   virtual dispatch.
//! - `Node<T>` wraps every AST node with source location info.
//! - `Identifier` uses `Cell` fields for scope analysis results that are
//!   written after parsing (by the scope collector).
//! - Operator enums use `#[repr(u8)]` with values matching the C++ enums
//!   for trivial FFI conversion.
//! - `ScopeData` replaces the C++ `ScopeNode` base class, carried by
//!   block-like constructs (Program, BlockStatement, FunctionBody, etc.).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

// =============================================================================
// Source location
// =============================================================================

/// UTF-16 encoded string (matches C++ Utf16String / Utf16FlyString).
pub type Utf16String = Vec<u16>;

#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    pub line: u32,
    pub column: u32,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SourceRange {
    pub start: Position,
    pub end: Position,
}

// =============================================================================
// Node wrapper
// =============================================================================

/// Every AST node wraps its payload with source location.
pub struct Node<T> {
    pub range: SourceRange,
    pub inner: T,
}

impl<T: Clone> Clone for Node<T> {
    fn clone(&self) -> Self {
        Self {
            range: self.range,
            inner: self.inner.clone(),
        }
    }
}

/// Expression node: `Node<Expression>`.
pub type Expr = Node<Expression>;

/// Statement node: `Node<Statement>`.
pub type Stmt = Node<Statement>;

impl<T> Node<T> {
    pub fn new(range: SourceRange, inner: T) -> Self {
        Self { range, inner }
    }
}

// =============================================================================
// Operator enums — values match C++ AST.h enums
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BinaryOp {
    Addition = 0,
    Subtraction = 1,
    Multiplication = 2,
    Division = 3,
    Modulo = 4,
    Exponentiation = 5,
    StrictlyEquals = 6,
    StrictlyInequals = 7,
    LooselyEquals = 8,
    LooselyInequals = 9,
    GreaterThan = 10,
    GreaterThanEquals = 11,
    LessThan = 12,
    LessThanEquals = 13,
    BitwiseAnd = 14,
    BitwiseOr = 15,
    BitwiseXor = 16,
    LeftShift = 17,
    RightShift = 18,
    UnsignedRightShift = 19,
    In = 20,
    InstanceOf = 21,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LogicalOp {
    And = 0,
    Or = 1,
    NullishCoalescing = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UnaryOp {
    BitwiseNot = 0,
    Not = 1,
    Plus = 2,
    Minus = 3,
    Typeof = 4,
    Void = 5,
    Delete = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateOp {
    Increment = 0,
    Decrement = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AssignmentOp {
    Assignment = 0,
    AdditionAssignment = 1,
    SubtractionAssignment = 2,
    MultiplicationAssignment = 3,
    DivisionAssignment = 4,
    ModuloAssignment = 5,
    ExponentiationAssignment = 6,
    BitwiseAndAssignment = 7,
    BitwiseOrAssignment = 8,
    BitwiseXorAssignment = 9,
    LeftShiftAssignment = 10,
    RightShiftAssignment = 11,
    UnsignedRightShiftAssignment = 12,
    AndAssignment = 13,
    OrAssignment = 14,
    NullishAssignment = 15,
}

// =============================================================================
// Kind enums
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DeclarationKind {
    Var = 1,
    Let = 2,
    Const = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionKind {
    Normal = 0,
    Generator = 1,
    Async = 2,
    AsyncGenerator = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramType {
    Script,
    Module,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaPropertyType {
    NewTarget,
    ImportMeta,
}

// =============================================================================
// Identifier
// =============================================================================

/// Scope analysis result: how this identifier is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalType {
    None,
    Argument,
    Variable,
}

/// Declaration kind as seen from an identifier reference.
/// Values match C++ `DeclarationKind` (with None=0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IdentDeclarationKind {
    None = 0,
    Var = 1,
    Let = 2,
    Const = 3,
}

/// An identifier reference or binding name.
///
/// Scope analysis results are stored in `Cell` fields because the scope
/// collector writes them after parsing through shared references.
#[derive(Clone)]
pub struct Identifier {
    pub range: SourceRange,
    pub name: Utf16String,
    // Scope analysis results — set by scope collector after parsing.
    pub local_type: Cell<LocalType>,
    pub local_index: Cell<u32>,
    pub is_global: Cell<bool>,
    pub is_inside_scope_with_eval: Cell<bool>,
    pub declaration_kind: Cell<IdentDeclarationKind>,
}

impl Identifier {
    pub fn new(range: SourceRange, name: Utf16String) -> Self {
        Self {
            range,
            name,
            local_type: Cell::new(LocalType::None),
            local_index: Cell::new(0),
            is_global: Cell::new(false),
            is_inside_scope_with_eval: Cell::new(false),
            declaration_kind: Cell::new(IdentDeclarationKind::None),
        }
    }

    pub fn is_local(&self) -> bool {
        self.local_type.get() != LocalType::None
    }
}

/// A private identifier (`#name`).
#[derive(Clone)]
pub struct PrivateIdentifier {
    pub range: SourceRange,
    pub name: Utf16String,
}

// =============================================================================
// Function support types
// =============================================================================

/// Parsing insights collected during function body parsing.
///
/// Without scope collector, these default to `false`. They'll be
/// populated properly once the Rust scope collector is implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct FunctionParsingInsights {
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
    pub might_need_arguments_object: bool,
}

/// A formal parameter in a function declaration/expression.
#[derive(Clone)]
pub struct FunctionParameter {
    pub binding: FunctionParameterBinding,
    pub default_value: Option<Expr>,
    pub is_rest: bool,
}

#[derive(Clone)]
pub enum FunctionParameterBinding {
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

/// Shared data for FunctionDeclaration and FunctionExpression.
#[derive(Clone)]
pub struct FunctionData {
    pub name: Option<Rc<Identifier>>,
    pub source_text_start: u32,
    pub source_text_end: u32,
    pub body: Box<Stmt>,
    pub parameters: Vec<FunctionParameter>,
    pub function_length: i32,
    pub kind: FunctionKind,
    pub is_strict_mode: bool,
    pub is_arrow_function: bool,
    pub parsing_insights: FunctionParsingInsights,
    pub is_hoisted: bool,
}

// =============================================================================
// Class support types
// =============================================================================

/// Shared data for ClassDeclaration and ClassExpression.
#[derive(Clone)]
pub struct ClassData {
    pub name: Option<Rc<Identifier>>,
    pub source_text_start: u32,
    pub source_text_end: u32,
    pub constructor: Option<Box<Expr>>,
    pub super_class: Option<Box<Expr>>,
    pub elements: Vec<Node<ClassElement>>,
}

#[derive(Clone)]
pub enum ClassElement {
    Method {
        key: Box<Expr>,
        function: Box<Expr>,
        kind: ClassMethodKind,
        is_static: bool,
    },
    Field {
        key: Box<Expr>,
        initializer: Option<Box<Expr>>,
        is_static: bool,
    },
    StaticInitializer {
        body: Box<Stmt>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClassMethodKind {
    Method = 0,
    Getter = 1,
    Setter = 2,
}

// =============================================================================
// Binding pattern types
// =============================================================================

/// Destructuring pattern for array/object bindings.
#[derive(Clone)]
pub struct BindingPattern {
    pub kind: BindingPatternKind,
    pub entries: Vec<BindingEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingPatternKind {
    Array,
    Object,
}

#[derive(Clone)]
pub struct BindingEntry {
    pub name: BindingEntryName,
    pub alias: BindingEntryAlias,
    pub initializer: Option<Expr>,
    pub is_rest: bool,
}

/// The "name" part of a binding entry.
/// - `Empty`: elision in array patterns (`[, , x]`)
/// - `Identifier`: object property shorthand (`{ x }`)
/// - `Expression`: computed property key (`{ [expr]: x }`)
#[derive(Clone)]
pub enum BindingEntryName {
    Empty,
    Identifier(Rc<Identifier>),
    Expression(Box<Expr>),
}

/// The "alias" (target) of a binding entry.
/// - `Empty`: name is the binding target (`{ x }` — x is both name and alias)
/// - `Identifier`: simple binding (`{ x: y }`)
/// - `BindingPattern`: nested destructuring (`{ x: { a, b } }`)
/// - `MemberExpression`: assignment target (`{ x: obj.prop }`)
#[derive(Clone)]
pub enum BindingEntryAlias {
    Empty,
    Identifier(Rc<Identifier>),
    BindingPattern(Box<BindingPattern>),
    MemberExpression(Box<Expr>),
}

// =============================================================================
// Variable declaration types
// =============================================================================

#[derive(Clone)]
pub struct VariableDeclarator {
    pub range: SourceRange,
    pub target: VariableDeclaratorTarget,
    pub init: Option<Expr>,
}

#[derive(Clone)]
pub enum VariableDeclaratorTarget {
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

// =============================================================================
// Object literal types
// =============================================================================

#[derive(Clone)]
pub struct ObjectProperty {
    pub range: SourceRange,
    pub property_type: ObjectPropertyType,
    pub key: Box<Expr>,
    pub value: Option<Box<Expr>>,
    pub is_method: bool,
    pub is_computed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectPropertyType {
    KeyValue = 0,
    Getter = 1,
    Setter = 2,
    Spread = 3,
    ProtoSetter = 4,
}

// =============================================================================
// Call expression types
// =============================================================================

#[derive(Clone)]
pub struct CallArgument {
    pub value: Expr,
    pub is_spread: bool,
}

#[derive(Clone)]
pub struct CallExpressionData {
    pub callee: Box<Expr>,
    pub arguments: Vec<CallArgument>,
    pub is_parenthesized: bool,
    pub is_inside_parens: bool,
}

#[derive(Clone)]
pub struct SuperCallData {
    pub arguments: Vec<CallArgument>,
    pub is_synthetic: bool,
}

// =============================================================================
// Optional chain types
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalChainMode {
    Optional,
    NotOptional,
}

#[derive(Clone)]
pub enum OptionalChainReference {
    Call {
        arguments: Vec<CallArgument>,
        mode: OptionalChainMode,
    },
    ComputedReference {
        expression: Box<Expr>,
        mode: OptionalChainMode,
    },
    MemberReference {
        identifier: Rc<Identifier>,
        mode: OptionalChainMode,
    },
    PrivateMemberReference {
        private_identifier: PrivateIdentifier,
        mode: OptionalChainMode,
    },
}

// =============================================================================
// Template literal types
// =============================================================================

#[derive(Clone)]
pub struct TemplateLiteralData {
    pub expressions: Vec<Expr>,
    pub raw_strings: Vec<Utf16String>,
}

// =============================================================================
// RegExp literal
// =============================================================================

#[derive(Clone)]
pub struct RegExpLiteralData {
    pub pattern: Utf16String,
    pub flags: Utf16String,
}

// =============================================================================
// Try/Catch types
// =============================================================================

#[derive(Clone)]
pub struct TryStatementData {
    pub block: Box<Stmt>,
    pub handler: Option<CatchClause>,
    pub finalizer: Option<Box<Stmt>>,
}

#[derive(Clone)]
pub struct CatchClause {
    pub range: SourceRange,
    pub parameter: CatchParameter,
    pub body: Box<Stmt>,
}

#[derive(Clone)]
pub enum CatchParameter {
    None,
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

// =============================================================================
// Switch types
// =============================================================================

#[derive(Clone)]
pub struct SwitchStatementData {
    pub scope: Rc<RefCell<ScopeData>>,
    pub discriminant: Box<Expr>,
    pub cases: Vec<SwitchCase>,
}

#[derive(Clone)]
pub struct SwitchCase {
    pub range: SourceRange,
    pub scope: Rc<RefCell<ScopeData>>,
    pub test: Option<Expr>,
}

// =============================================================================
// Module types (import/export)
// =============================================================================

#[derive(Clone)]
pub struct ModuleRequest {
    pub module_specifier: Utf16String,
    pub attributes: Vec<ImportAttribute>,
}

#[derive(Clone)]
pub struct ImportAttribute {
    pub key: Utf16String,
    pub value: Utf16String,
}

#[derive(Clone)]
pub struct ImportEntry {
    /// `None` means namespace import (`import * as x`).
    pub import_name: Option<Utf16String>,
    pub local_name: Utf16String,
}

#[derive(Clone)]
pub struct ImportStatementData {
    pub module_request: ModuleRequest,
    pub entries: Vec<ImportEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExportEntryKind {
    NamedExport = 0,
    ModuleRequestAll = 1,
    ModuleRequestAllButDefault = 2,
    EmptyNamedExport = 3,
}

#[derive(Clone)]
pub struct ExportEntry {
    pub kind: ExportEntryKind,
    pub export_name: Option<Utf16String>,
    pub local_or_import_name: Option<Utf16String>,
}

#[derive(Clone)]
pub struct ExportStatementData {
    pub statement: Option<Box<Stmt>>,
    pub entries: Vec<ExportEntry>,
    pub is_default_export: bool,
    pub module_request: Option<ModuleRequest>,
}

// =============================================================================
// For-in/of LHS
// =============================================================================

/// Left-hand side of for-in, for-of, for-await-of.
#[derive(Clone)]
pub enum ForInOfLhs {
    /// A variable declaration (`for (let x of ...)`)
    Declaration(Box<Stmt>),
    /// An expression (`for (x in obj)`)
    Expression(Box<Expr>),
    /// A binding pattern (`for ({a, b} of ...)`)
    Pattern(BindingPattern),
}

// =============================================================================
// Assignment LHS
// =============================================================================

/// Left-hand side of an assignment expression.
#[derive(Clone)]
pub enum AssignmentLhs {
    Expression(Box<Expr>),
    Pattern(BindingPattern),
}

// =============================================================================
// Scope data (replaces C++ ScopeNode base class)
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LocalVarKind {
    Var = 0,
    LetOrConst = 1,
    Function = 2,
    ArgumentsObject = 3,
    CatchClauseParameter = 4,
}

#[derive(Clone)]
pub struct LocalVariable {
    pub name: Utf16String,
    pub kind: LocalVarKind,
}

/// Data shared by all scope-bearing nodes (Program, BlockStatement,
/// FunctionBody, SwitchStatement, SwitchCase).
#[derive(Clone)]
pub struct ScopeData {
    pub children: Vec<Stmt>,
    pub local_variables: Vec<LocalVariable>,
    pub function_scope_data: Option<Box<FunctionScopeData>>,
    pub hoisted_functions: Vec<usize>,
    /// Function names hoisted from inner blocks via Annex B.3.3.
    /// The FDI should create `var` bindings initialized to `undefined`
    /// for each name.
    pub annexb_function_names: Vec<Vec<u16>>,
    // Scope analysis insights, written by the scope collector after analyze().
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
    pub contains_access_to_arguments_object: bool,
}

impl ScopeData {
    pub fn new_shared() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            children: Vec::new(),
            local_variables: Vec::new(),
            function_scope_data: None,
            hoisted_functions: Vec::new(),
            annexb_function_names: Vec::new(),
            uses_this: false,
            uses_this_from_environment: false,
            contains_direct_call_to_eval: false,
            contains_access_to_arguments_object: false,
        }))
    }

    pub fn shared_with_children(children: Vec<Stmt>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            children,
            local_variables: Vec::new(),
            function_scope_data: None,
            hoisted_functions: Vec::new(),
            annexb_function_names: Vec::new(),
            uses_this: false,
            uses_this_from_environment: false,
            contains_direct_call_to_eval: false,
            contains_access_to_arguments_object: false,
        }))
    }
}

/// Scope analysis data for function bodies, populated by the scope collector.
#[derive(Clone)]
pub struct FunctionScopeData {
    pub functions_to_initialize: Vec<FunctionToInit>,
    pub vars_to_initialize: Vec<VarToInit>,
    pub var_names: Vec<Utf16String>,
    pub has_function_named_arguments: bool,
    pub has_argument_parameter: bool,
    pub has_lexically_declared_arguments: bool,
    pub non_local_var_count: usize,
    pub non_local_var_count_for_parameter_expressions: usize,
}

/// Reference to a function declaration that needs hoisting/initialization.
/// Stores the index within the parent ScopeData.children.
#[derive(Clone)]
pub struct FunctionToInit {
    pub child_index: usize,
}

/// A `var` binding that needs initialization during function entry.
#[derive(Clone)]
pub struct VarToInit {
    pub name: Utf16String,
    pub is_parameter: bool,
    pub is_function_name: bool,
    /// If the scope analysis optimized this var to a local, stores the operand type and index.
    pub local: Option<(LocalType, u32)>,
}

// =============================================================================
// Expression enum
// =============================================================================

#[derive(Clone)]
pub enum Expression {
    // Literals
    NumericLiteral(f64),
    StringLiteral(Utf16String),
    BooleanLiteral(bool),
    NullLiteral,
    BigIntLiteral(String),
    RegExpLiteral(RegExpLiteralData),

    // Identifiers
    Identifier(Rc<Identifier>),
    PrivateIdentifier(PrivateIdentifier),

    // Operators
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Logical {
        op: LogicalOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Update {
        op: UpdateOp,
        argument: Box<Expr>,
        prefixed: bool,
    },
    Assignment {
        op: AssignmentOp,
        lhs: AssignmentLhs,
        rhs: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
    Sequence(Vec<Expr>),

    // Member access
    Member {
        object: Box<Expr>,
        property: Box<Expr>,
        computed: bool,
    },
    OptionalChain {
        base: Box<Expr>,
        references: Vec<OptionalChainReference>,
    },

    // Calls
    Call(CallExpressionData),
    New(CallExpressionData),
    SuperCall(SuperCallData),

    // Spread
    Spread(Box<Expr>),

    // This / Super
    This,
    Super,

    // Functions
    Function(Box<FunctionData>),

    // Classes
    Class(Box<ClassData>),

    // Collections
    Array(Vec<Option<Expr>>),
    Object(Vec<ObjectProperty>),

    // Templates
    TemplateLiteral(TemplateLiteralData),
    TaggedTemplateLiteral {
        tag: Box<Expr>,
        template_literal: Box<Expr>,
    },

    // Meta
    MetaProperty(MetaPropertyType),
    ImportCall {
        specifier: Box<Expr>,
        options: Option<Box<Expr>>,
    },

    // Async / Generator
    Yield {
        argument: Option<Box<Expr>>,
        is_yield_from: bool,
    },
    Await(Box<Expr>),

    // Error recovery
    Error,
}

// =============================================================================
// Statement enum
// =============================================================================

#[derive(Clone)]
pub enum Statement {
    // Basic
    Empty,
    Error,
    Expression(Box<Expr>),
    Debugger,

    // Blocks (carry ScopeData like C++ ScopeNode)
    Block(Rc<RefCell<ScopeData>>),
    FunctionBody {
        scope: Rc<RefCell<ScopeData>>,
        in_strict_mode: bool,
    },
    Program(ProgramData),

    // Control flow
    If {
        predicate: Box<Expr>,
        consequent: Box<Stmt>,
        alternate: Option<Box<Stmt>>,
    },
    While {
        test: Box<Expr>,
        body: Box<Stmt>,
    },
    DoWhile {
        test: Box<Expr>,
        body: Box<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        test: Option<Box<Expr>>,
        update: Option<Box<Expr>>,
        body: Box<Stmt>,
    },
    ForIn {
        lhs: ForInOfLhs,
        rhs: Box<Expr>,
        body: Box<Stmt>,
    },
    ForOf {
        lhs: ForInOfLhs,
        rhs: Box<Expr>,
        body: Box<Stmt>,
    },
    ForAwaitOf {
        lhs: ForInOfLhs,
        rhs: Box<Expr>,
        body: Box<Stmt>,
    },
    Switch(SwitchStatementData),
    With {
        object: Box<Expr>,
        body: Box<Stmt>,
    },
    Labelled {
        label: Utf16String,
        item: Box<Stmt>,
    },

    // Jumps
    Break {
        target_label: Option<Utf16String>,
    },
    Continue {
        target_label: Option<Utf16String>,
    },
    Return(Option<Box<Expr>>),
    Throw(Box<Expr>),
    Try(TryStatementData),

    // Declarations
    VariableDeclaration {
        kind: DeclarationKind,
        declarations: Vec<VariableDeclarator>,
    },
    UsingDeclaration {
        declarations: Vec<VariableDeclarator>,
    },
    FunctionDeclaration(Box<FunctionData>),
    ClassDeclaration(Box<ClassData>),
    ErrorDeclaration,

    // Module
    Import(ImportStatementData),
    Export(ExportStatementData),

    // Special
    ClassFieldInitializer {
        expression: Box<Expr>,
        field_name: Utf16String,
    },
}

// =============================================================================
// Program data
// =============================================================================

#[derive(Clone)]
pub struct ProgramData {
    pub scope: Rc<RefCell<ScopeData>>,
    pub program_type: ProgramType,
    pub is_strict_mode: bool,
    pub has_top_level_await: bool,
}
