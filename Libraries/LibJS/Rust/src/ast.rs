/*
 * Copyright (c) 2026, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Rust AST types for JavaScript.
//!
//! This module defines the Abstract Syntax Tree using idiomatic Rust enums.
//! Every node carries a `SourceRange` for error messages and source maps.
//!
//! ## Design
//!
//! - `ExpressionKind` and `StatementKind` are flat enums — pattern matching
//!   replaces virtual dispatch.
//! - `Node<T>` wraps every AST node with source location info.
//! - `Identifier` uses `Cell` fields for scope analysis results that are
//!   written after parsing (by the scope collector).
//! - Operator enums use `#[repr(u8)]` with ABI-compatible values for
//!   trivial FFI conversion.
//! - `ScopeData` is carried by block-like constructs (Program,
//!   BlockStatement, FunctionBody, etc.) and holds scope analysis results.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

// =============================================================================
// Source location
// =============================================================================

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
#[derive(Clone)]
pub struct Node<T> {
    pub range: SourceRange,
    pub inner: T,
}

pub type Expression = Node<ExpressionKind>;

pub type Statement = Node<StatementKind>;

impl<T> Node<T> {
    pub fn new(range: SourceRange, inner: T) -> Self {
        Self { range, inner }
    }
}

// =============================================================================
// Operator enums — values are ABI-compatible for FFI
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

impl FunctionKind {
    pub fn from_async_generator(is_async: bool, is_generator: bool) -> Self {
        match (is_async, is_generator) {
            (true, true) => Self::AsyncGenerator,
            (true, false) => Self::Async,
            (false, true) => Self::Generator,
            (false, false) => Self::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProgramType {
    Script = 0,
    Module = 1,
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
    Argument,
    Variable,
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
    pub local_type: Cell<Option<LocalType>>,
    pub local_index: Cell<u32>,
    pub is_global: Cell<bool>,
    pub is_inside_scope_with_eval: Cell<bool>,
    pub declaration_kind: Cell<Option<DeclarationKind>>,
}

impl Identifier {
    pub fn new(range: SourceRange, name: Utf16String) -> Self {
        Self {
            range,
            name,
            local_type: Cell::new(None),
            local_index: Cell::new(0),
            is_global: Cell::new(false),
            is_inside_scope_with_eval: Cell::new(false),
            declaration_kind: Cell::new(None),
        }
    }

    pub fn is_local(&self) -> bool {
        self.local_type.get().is_some()
    }
}

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
/// The scope collector populates `uses_this`, `uses_this_from_environment`,
/// and `contains_direct_call_to_eval` during scope analysis.
/// `might_need_arguments_object` is set by the parser during body parsing.
#[derive(Clone, Copy, Debug, Default)]
pub struct FunctionParsingInsights {
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
    pub might_need_arguments_object: bool,
}

#[derive(Clone)]
pub struct FunctionParameter {
    pub binding: FunctionParameterBinding,
    pub default_value: Option<Expression>,
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
    pub body: Box<Statement>,
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
    pub constructor: Option<Box<Expression>>,
    pub super_class: Option<Box<Expression>>,
    pub elements: Vec<Node<ClassElement>>,
}

#[derive(Clone)]
pub enum ClassElement {
    Method {
        key: Box<Expression>,
        function: Box<Expression>,
        kind: ClassMethodKind,
        is_static: bool,
    },
    Field {
        key: Box<Expression>,
        initializer: Option<Box<Expression>>,
        is_static: bool,
    },
    StaticInitializer {
        body: Box<Statement>,
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
    pub name: Option<BindingEntryName>,
    pub alias: Option<BindingEntryAlias>,
    pub initializer: Option<Expression>,
    pub is_rest: bool,
}

/// The "name" part of a binding entry.
/// - `None`: elision in array patterns (`[, , x]`)
/// - `Identifier`: object property shorthand (`{ x }`)
/// - `Expression`: computed property key (`{ [expression]: x }`)
#[derive(Clone)]
pub enum BindingEntryName {
    Identifier(Rc<Identifier>),
    Expression(Box<Expression>),
}

/// The "alias" (target) of a binding entry.
/// - `None`: name is the binding target (`{ x }` — x is both name and alias)
/// - `Identifier`: simple binding (`{ x: y }`)
/// - `BindingPattern`: nested destructuring (`{ x: { a, b } }`)
/// - `MemberExpression`: assignment target (`{ x: obj.property }`)
#[derive(Clone)]
pub enum BindingEntryAlias {
    Identifier(Rc<Identifier>),
    BindingPattern(Box<BindingPattern>),
    MemberExpression(Box<Expression>),
}

// =============================================================================
// Variable declaration types
// =============================================================================

#[derive(Clone)]
pub struct VariableDeclarator {
    pub range: SourceRange,
    pub target: VariableDeclaratorTarget,
    pub init: Option<Expression>,
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
    pub key: Box<Expression>,
    pub value: Option<Box<Expression>>,
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
    pub value: Expression,
    pub is_spread: bool,
}

#[derive(Clone)]
pub struct CallExpressionData {
    pub callee: Box<Expression>,
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
        expression: Box<Expression>,
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
    pub expressions: Vec<Expression>,
    pub raw_strings: Vec<Utf16String>,
}

// =============================================================================
// RegExp literal
// =============================================================================

/// RAII wrapper for a compiled regex handle from C++.
/// Takes ownership via `take()` and frees via FFI on drop.
pub struct CompiledRegex(Cell<*mut c_void>);

extern "C" {
    fn rust_free_compiled_regex(ptr: *mut c_void);
}

impl CompiledRegex {
    pub fn new(ptr: *mut c_void) -> Self {
        Self(Cell::new(ptr))
    }

    /// Take ownership of the compiled regex handle, leaving null behind.
    pub fn take(&self) -> *mut c_void {
        self.0.replace(std::ptr::null_mut())
    }
}

impl Drop for CompiledRegex {
    fn drop(&mut self) {
        let ptr = self.0.get();
        if !ptr.is_null() {
            unsafe { rust_free_compiled_regex(ptr) };
        }
    }
}

pub struct RegExpLiteralData {
    pub pattern: Utf16String,
    pub flags: Utf16String,
    pub compiled_regex: CompiledRegex,
}

impl Clone for RegExpLiteralData {
    /// Clone transfers ownership of the compiled regex handle from the
    /// original to the clone (leaving the original's handle null). This
    /// supports the SFD lazy compilation path where the AST is cloned and
    /// the clone is the one that will actually be compiled later.
    fn clone(&self) -> Self {
        Self {
            pattern: self.pattern.clone(),
            flags: self.flags.clone(),
            compiled_regex: CompiledRegex::new(self.compiled_regex.take()),
        }
    }
}

// =============================================================================
// Try/Catch types
// =============================================================================

#[derive(Clone)]
pub struct TryStatementData {
    pub block: Box<Statement>,
    pub handler: Option<CatchClause>,
    pub finalizer: Option<Box<Statement>>,
}

#[derive(Clone)]
pub struct CatchClause {
    pub range: SourceRange,
    pub parameter: CatchParameter,
    pub body: Box<Statement>,
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
    pub discriminant: Box<Expression>,
    pub cases: Vec<SwitchCase>,
}

#[derive(Clone)]
pub struct SwitchCase {
    pub range: SourceRange,
    pub scope: Rc<RefCell<ScopeData>>,
    pub test: Option<Expression>,
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
    pub statement: Option<Box<Statement>>,
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
    Declaration(Box<Statement>),
    /// An expression (`for (x in obj)`)
    Expression(Box<Expression>),
    /// A binding pattern (`for ({a, b} of ...)`)
    Pattern(BindingPattern),
}

// =============================================================================
// Assignment LHS
// =============================================================================

#[derive(Clone)]
pub enum AssignmentLhs {
    Expression(Box<Expression>),
    Pattern(BindingPattern),
}

// =============================================================================
// Scope data
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
#[derive(Clone, Default)]
pub struct ScopeData {
    pub children: Vec<Statement>,
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
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn shared_with_children(children: Vec<Statement>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            children,
            ..Default::default()
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

/// A resolved local binding: the operand type and index assigned by scope analysis.
#[derive(Clone, Copy, Debug)]
pub struct LocalBinding {
    pub local_type: LocalType,
    pub index: u32,
}

/// A `var` binding that needs initialization during function entry.
#[derive(Clone)]
pub struct VarToInit {
    pub name: Utf16String,
    pub is_parameter: bool,
    pub is_function_name: bool,
    /// If the scope analysis optimized this var to a local, stores the binding info.
    pub local: Option<LocalBinding>,
}

// =============================================================================
// Expression enum
// =============================================================================

#[derive(Clone)]
pub enum ExpressionKind {
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
        lhs: Box<Expression>,
        rhs: Box<Expression>,
    },
    Logical {
        op: LogicalOp,
        lhs: Box<Expression>,
        rhs: Box<Expression>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expression>,
    },
    Update {
        op: UpdateOp,
        argument: Box<Expression>,
        prefixed: bool,
    },
    Assignment {
        op: AssignmentOp,
        lhs: AssignmentLhs,
        rhs: Box<Expression>,
    },
    Conditional {
        test: Box<Expression>,
        consequent: Box<Expression>,
        alternate: Box<Expression>,
    },
    Sequence(Vec<Expression>),

    // Member access
    Member {
        object: Box<Expression>,
        property: Box<Expression>,
        computed: bool,
    },
    OptionalChain {
        base: Box<Expression>,
        references: Vec<OptionalChainReference>,
    },

    // Calls
    Call(CallExpressionData),
    New(CallExpressionData),
    SuperCall(SuperCallData),

    // Spread
    Spread(Box<Expression>),

    // This / Super
    This,
    Super,

    // Functions
    Function(Box<FunctionData>),

    // Classes
    Class(Box<ClassData>),

    // Collections
    Array(Vec<Option<Expression>>),
    Object(Vec<ObjectProperty>),

    // Templates
    TemplateLiteral(TemplateLiteralData),
    TaggedTemplateLiteral {
        tag: Box<Expression>,
        template_literal: Box<Expression>,
    },

    // Meta
    MetaProperty(MetaPropertyType),
    ImportCall {
        specifier: Box<Expression>,
        options: Option<Box<Expression>>,
    },

    // Async / Generator
    Yield {
        argument: Option<Box<Expression>>,
        is_yield_from: bool,
    },
    Await(Box<Expression>),

    // Error recovery
    Error,
}

// =============================================================================
// Statement enum
// =============================================================================

#[derive(Clone)]
pub enum StatementKind {
    // Basic
    Empty,
    Error,
    Expression(Box<Expression>),
    Debugger,

    // Blocks (carry ScopeData for scope analysis)
    Block(Rc<RefCell<ScopeData>>),
    FunctionBody {
        scope: Rc<RefCell<ScopeData>>,
        in_strict_mode: bool,
    },
    Program(ProgramData),

    // Control flow
    If {
        test: Box<Expression>,
        consequent: Box<Statement>,
        alternate: Option<Box<Statement>>,
    },
    While {
        test: Box<Expression>,
        body: Box<Statement>,
    },
    DoWhile {
        test: Box<Expression>,
        body: Box<Statement>,
    },
    For {
        init: Option<Box<Statement>>,
        test: Option<Box<Expression>>,
        update: Option<Box<Expression>>,
        body: Box<Statement>,
    },
    ForIn {
        lhs: ForInOfLhs,
        rhs: Box<Expression>,
        body: Box<Statement>,
    },
    ForOf {
        lhs: ForInOfLhs,
        rhs: Box<Expression>,
        body: Box<Statement>,
    },
    ForAwaitOf {
        lhs: ForInOfLhs,
        rhs: Box<Expression>,
        body: Box<Statement>,
    },
    Switch(SwitchStatementData),
    With {
        object: Box<Expression>,
        body: Box<Statement>,
    },
    Labelled {
        label: Utf16String,
        item: Box<Statement>,
    },

    // Jumps
    Break {
        target_label: Option<Utf16String>,
    },
    Continue {
        target_label: Option<Utf16String>,
    },
    Return(Option<Box<Expression>>),
    Throw(Box<Expression>),
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
        expression: Box<Expression>,
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
