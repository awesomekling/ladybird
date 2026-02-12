/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Bytecode generator.
//!
//! This module contains the `Generator` struct which manages all state
//! needed for bytecode generation from the Rust AST. It mirrors the
//! C++ `Bytecode::Generator` class.

use std::collections::HashMap;

use super::basic_block::{BasicBlock, SourceMapEntry};
use super::instruction::Instruction;
use super::operand::*;

/// Identifies an operand that auto-frees its register when the last
/// clone is dropped.
///
/// In C++, `ScopedOperand` is a ref-counted wrapper. In Rust, we use
/// a simple `Rc`-based approach. When the last clone drops and the
/// operand is a non-reserved register, it gets freed back to the pool.
#[derive(Debug, Clone)]
pub struct ScopedOperand {
    inner: std::rc::Rc<ScopedOperandInner>,
}

struct ScopedOperandInner {
    operand: Operand,
    generator: *mut Generator,
}

impl std::fmt::Debug for ScopedOperandInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ScopedOperandInner({:?})", self.operand)
    }
}

impl Drop for ScopedOperandInner {
    fn drop(&mut self) {
        if self.generator.is_null() {
            return;
        }
        let gen = unsafe { &mut *self.generator };
        if gen.finished {
            return;
        }
        if self.operand.is_register() && self.operand.index() >= Register::RESERVED_COUNT {
            gen.free_registers.push(Register(self.operand.index()));
        }
    }
}

impl ScopedOperand {
    pub fn operand(&self) -> Operand {
        self.inner.operand
    }
}

impl PartialEq for ScopedOperand {
    fn eq(&self, other: &Self) -> bool {
        self.inner.operand == other.inner.operand
    }
}

/// Function kind, matching C++ `FunctionKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Normal,
    Generator,
    Async,
    AsyncGenerator,
}

/// Block boundary types for unwind tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockBoundaryType {
    Break,
    Continue,
    ReturnToFinally,
    LeaveFinally,
    LeaveLexicalEnvironment,
}

/// A break/continue scope with its target label and language labels.
pub struct LabelableScope {
    pub bytecode_target: Label,
    pub language_label_set: Vec<Vec<u16>>,
    pub completion_register: Option<ScopedOperand>,
}

/// Codegen-time state for a try/finally scope.
#[derive(Clone)]
pub struct FinallyContext {
    pub completion_type: ScopedOperand,
    pub completion_value: ScopedOperand,
    pub finally_body: Label,
    pub exception_preamble: Label,
    pub parent: Option<Box<FinallyContext>>,
    pub registered_jumps: Vec<FinallyJump>,
    pub next_jump_index: i32,
    pub lexical_environment_at_entry: Option<ScopedOperand>,
}

impl FinallyContext {
    pub const NORMAL: i32 = 0;
    pub const THROW: i32 = 1;
    pub const RETURN: i32 = 2;
    pub const FIRST_JUMP_INDEX: i32 = 3;
}

/// A break/continue target registered with a FinallyContext.
#[derive(Clone)]
pub struct FinallyJump {
    pub index: i32,
    pub target: Label,
}

/// A local variable name with metadata.
#[derive(Debug, Clone)]
pub struct LocalVariable {
    pub name: Vec<u16>,
    pub is_lexically_declared: bool,
    pub is_initialized_during_declaration_instantiation: bool,
}

/// The bytecode generator.
///
/// Manages all state needed for compiling a Rust AST into bytecode.
/// Mirrors the C++ `Bytecode::Generator` class.
pub struct Generator {
    // --- Basic block management ---
    pub basic_blocks: Vec<BasicBlock>,
    current_block_index: usize,
    next_block_id: u32,

    // --- Register allocation ---
    next_register: u32,
    free_registers: Vec<Register>,

    // --- Constant pool ---
    pub constants: Vec<ConstantValue>,

    // Cached constants for deduplication
    true_constant: Option<ScopedOperand>,
    false_constant: Option<ScopedOperand>,
    null_constant: Option<ScopedOperand>,
    undefined_constant: Option<ScopedOperand>,
    empty_constant: Option<ScopedOperand>,
    int32_constants: HashMap<i32, ScopedOperand>,
    string_constants: HashMap<Vec<u16>, ScopedOperand>,

    // --- String/identifier/property tables ---
    // These are Vec<Vec<u16>> (UTF-16 strings) that will be passed to
    // C++ via FFI when creating the Executable.
    pub string_table: Vec<Vec<u16>>,
    pub identifier_table: Vec<Vec<u16>>,
    pub property_key_table: Vec<Vec<u16>>,
    pub regex_table: Vec<(Vec<u16>, Vec<u16>)>, // (pattern, flags)

    // --- Scope/unwind state ---
    pub boundaries: Vec<BlockBoundaryType>,
    pub continuable_scopes: Vec<LabelableScope>,
    pub breakable_scopes: Vec<LabelableScope>,
    pub pending_labels: Vec<Vec<u16>>,
    pub lexical_environment_register_stack: Vec<ScopedOperand>,
    pub home_objects: Vec<ScopedOperand>,

    // --- Finally context ---
    pub current_finally_context: Option<Box<FinallyContext>>,

    // --- Various counters ---
    pub next_property_lookup_cache: u32,
    pub next_global_variable_cache: u32,
    pub next_template_object_cache: u32,
    pub next_object_shape_cache: u32,

    // --- Codegen state ---
    pub strict: bool,
    pub enclosing_function_kind: FunctionKind,
    pub local_variables: Vec<LocalVariable>,
    pub initialized_locals: Vec<bool>,

    /// When set, function/class expressions will use this as their `.name`.
    /// Set by assignment/declaration codegen, consumed by function expression codegen.
    pub pending_lhs_name: Option<IdentifierTableIndex>,

    // Source location tracking
    pub current_source_start: u32,
    pub current_source_end: u32,

    // --- Completion register ---
    pub current_completion_register: Option<ScopedOperand>,

    // --- Accumulator and this ---
    accumulator: ScopedOperand,
    this_value: ScopedOperand,

    // --- Shared function data ---
    // Opaque pointers to C++ SharedFunctionInstanceData objects.
    pub shared_function_data: Vec<*mut std::ffi::c_void>,

    // --- Class blueprints ---
    // Opaque pointers to heap-allocated C++ ClassBlueprint objects.
    // Ownership transfers to the Executable during creation.
    pub class_blueprints: Vec<*mut std::ffi::c_void>,

    // --- Length identifier cache ---
    pub length_identifier: Option<PropertyKeyTableIndex>,

    // --- Unwind context ---
    // When set, newly created basic blocks inherit this handler index.
    pub current_unwind_handler: Option<usize>,

    // --- Generator finished flag ---
    pub finished: bool,

    // --- FFI context ---
    // These are set by the top-level compiler and passed through for
    // creating SharedFunctionInstanceData via FFI callbacks.
    pub vm_ptr: *mut std::ffi::c_void,
    pub source_code_ptr: *const std::ffi::c_void,
    pub source: *const u16,
    pub source_len: usize,
}

impl Generator {
    /// Create a new bytecode generator.
    pub fn new() -> Self {
        // We need a self-referential structure for ScopedOperand.
        // Create the generator first, then fix up the operands.
        let mut gen = Self {
            basic_blocks: Vec::new(),
            current_block_index: 0,
            next_block_id: 1,
            next_register: Register::RESERVED_COUNT,
            free_registers: Vec::new(),
            constants: Vec::new(),
            true_constant: None,
            false_constant: None,
            null_constant: None,
            undefined_constant: None,
            empty_constant: None,
            int32_constants: HashMap::new(),
            string_constants: HashMap::new(),
            string_table: Vec::new(),
            identifier_table: Vec::new(),
            property_key_table: Vec::new(),
            regex_table: Vec::new(),
            boundaries: Vec::new(),
            continuable_scopes: Vec::new(),
            breakable_scopes: Vec::new(),
            pending_labels: Vec::new(),
            lexical_environment_register_stack: Vec::new(),
            home_objects: Vec::new(),
            current_finally_context: None,
            next_property_lookup_cache: 0,
            next_global_variable_cache: 0,
            next_template_object_cache: 0,
            next_object_shape_cache: 0,
            strict: false,
            enclosing_function_kind: FunctionKind::Normal,
            local_variables: Vec::new(),
            initialized_locals: Vec::new(),
            pending_lhs_name: None,
            current_source_start: 0,
            current_source_end: 0,
            current_completion_register: None,
            // Placeholder — will be fixed up below
            accumulator: ScopedOperand {
                inner: std::rc::Rc::new(ScopedOperandInner {
                    operand: Operand::register(Register::ACCUMULATOR),
                    generator: std::ptr::null_mut(),
                }),
            },
            this_value: ScopedOperand {
                inner: std::rc::Rc::new(ScopedOperandInner {
                    operand: Operand::register(Register::THIS_VALUE),
                    generator: std::ptr::null_mut(),
                }),
            },
            shared_function_data: Vec::new(),
            class_blueprints: Vec::new(),
            length_identifier: None,
            current_unwind_handler: None,
            finished: false,
            vm_ptr: std::ptr::null_mut(),
            source_code_ptr: std::ptr::null(),
            source: std::ptr::null(),
            source_len: 0,
        };

        // Fix up the self-referential pointers.
        let self_ptr = &mut gen as *mut Generator;
        let acc_inner = std::rc::Rc::get_mut(&mut gen.accumulator.inner).unwrap();
        acc_inner.generator = self_ptr;
        let this_inner = std::rc::Rc::get_mut(&mut gen.this_value.inner).unwrap();
        this_inner.generator = self_ptr;

        gen
    }

    // --- Function kind queries ---

    pub fn is_in_generator_function(&self) -> bool {
        matches!(self.enclosing_function_kind, FunctionKind::Generator | FunctionKind::AsyncGenerator)
    }

    pub fn is_in_async_function(&self) -> bool {
        matches!(self.enclosing_function_kind, FunctionKind::Async | FunctionKind::AsyncGenerator)
    }

    pub fn is_in_async_generator_function(&self) -> bool {
        self.enclosing_function_kind == FunctionKind::AsyncGenerator
    }

    pub fn is_in_generator_or_async_function(&self) -> bool {
        self.enclosing_function_kind != FunctionKind::Normal
    }

    // --- Register management ---

    /// Allocate a new register (or reuse a freed one).
    pub fn allocate_register(&mut self) -> ScopedOperand {
        let reg = if let Some(r) = self.free_registers.pop() {
            r
        } else {
            let r = Register(self.next_register);
            self.next_register += 1;
            r
        };
        self.scoped_operand(Operand::register(reg))
    }

    /// Get a ScopedOperand for a local variable.
    pub fn local(&mut self, index: u32) -> ScopedOperand {
        self.scoped_operand(Operand::local(index))
    }

    /// Get the accumulator register.
    pub fn accumulator(&self) -> ScopedOperand {
        self.accumulator.clone()
    }

    /// Get the this_value register.
    pub fn this_value(&self) -> ScopedOperand {
        self.this_value.clone()
    }

    pub fn scoped_operand(&mut self, operand: Operand) -> ScopedOperand {
        ScopedOperand {
            inner: std::rc::Rc::new(ScopedOperandInner {
                operand,
                generator: self as *mut Generator,
            }),
        }
    }

    // --- Constant pool ---

    fn append_constant(&mut self, value: ConstantValue) -> ScopedOperand {
        let index = self.constants.len() as u32;
        self.constants.push(value);
        self.scoped_operand(Operand::constant(index))
    }

    pub fn add_constant_number(&mut self, value: f64) -> ScopedOperand {
        // Deduplicate i32 values
        if value.fract() == 0.0 && value >= i32::MIN as f64 && value <= i32::MAX as f64 {
            let as_i32 = value as i32;
            if let Some(op) = self.int32_constants.get(&as_i32) {
                return op.clone();
            }
            let op = self.append_constant(ConstantValue::Number(value));
            self.int32_constants.insert(as_i32, op.clone());
            return op;
        }
        self.append_constant(ConstantValue::Number(value))
    }

    pub fn add_constant_boolean(&mut self, value: bool) -> ScopedOperand {
        if value {
            if let Some(op) = &self.true_constant {
                return op.clone();
            }
            let op = self.append_constant(ConstantValue::Boolean(true));
            self.true_constant = Some(op.clone());
            op
        } else {
            if let Some(op) = &self.false_constant {
                return op.clone();
            }
            let op = self.append_constant(ConstantValue::Boolean(false));
            self.false_constant = Some(op.clone());
            op
        }
    }

    pub fn add_constant_null(&mut self) -> ScopedOperand {
        if let Some(op) = &self.null_constant {
            return op.clone();
        }
        let op = self.append_constant(ConstantValue::Null);
        self.null_constant = Some(op.clone());
        op
    }

    pub fn add_constant_undefined(&mut self) -> ScopedOperand {
        if let Some(op) = &self.undefined_constant {
            return op.clone();
        }
        let op = self.append_constant(ConstantValue::Undefined);
        self.undefined_constant = Some(op.clone());
        op
    }

    pub fn add_constant_empty(&mut self) -> ScopedOperand {
        if let Some(op) = &self.empty_constant {
            return op.clone();
        }
        let op = self.append_constant(ConstantValue::Empty);
        self.empty_constant = Some(op.clone());
        op
    }

    pub fn add_constant_string(&mut self, value: Vec<u16>) -> ScopedOperand {
        if let Some(op) = self.string_constants.get(&value) {
            return op.clone();
        }
        let op = self.append_constant(ConstantValue::String(value.clone()));
        self.string_constants.insert(value, op.clone());
        op
    }

    pub fn add_constant_bigint(&mut self, value: String) -> ScopedOperand {
        self.append_constant(ConstantValue::BigInt(value))
    }

    // --- Table interning ---

    pub fn intern_string(&mut self, s: Vec<u16>) -> StringTableIndex {
        let index = self.string_table.len() as u32;
        self.string_table.push(s);
        StringTableIndex(index)
    }

    pub fn intern_identifier(&mut self, s: Vec<u16>) -> IdentifierTableIndex {
        let index = self.identifier_table.len() as u32;
        self.identifier_table.push(s);
        IdentifierTableIndex(index)
    }

    pub fn intern_property_key(&mut self, s: Vec<u16>) -> PropertyKeyTableIndex {
        let index = self.property_key_table.len() as u32;
        self.property_key_table.push(s);
        PropertyKeyTableIndex(index)
    }

    /// Register a SharedFunctionInstanceData (opaque C++ pointer) and return its index.
    pub fn register_shared_function_data(&mut self, ptr: *mut std::ffi::c_void) -> u32 {
        let index = self.shared_function_data.len() as u32;
        self.shared_function_data.push(ptr);
        index
    }

    /// Register a ClassBlueprint (opaque C++ pointer) and return its index.
    pub fn register_class_blueprint(&mut self, ptr: *mut std::ffi::c_void) -> u32 {
        let index = self.class_blueprints.len() as u32;
        self.class_blueprints.push(ptr);
        index
    }

    pub fn intern_regex(&mut self, pattern: Vec<u16>, flags: Vec<u16>) -> RegexTableIndex {
        let index = self.regex_table.len() as u32;
        self.regex_table.push((pattern, flags));
        RegexTableIndex(index)
    }

    // --- Basic block management ---

    /// Create a new basic block and return its index.
    pub fn make_block(&mut self) -> usize {
        let index = self.basic_blocks.len();
        let mut block = BasicBlock::new(index as u32);

        // Propagate exception handler from active unwind context.
        if let Some(handler) = self.current_unwind_handler {
            block.handler = Some(handler);
        }

        self.basic_blocks.push(block);
        self.next_block_id += 1;
        index
    }

    /// Switch emission to the given basic block.
    pub fn switch_to_basic_block(&mut self, block_index: usize) {
        self.current_block_index = block_index;
    }

    /// Get the current basic block's index.
    pub fn current_block_index(&self) -> usize {
        self.current_block_index
    }

    /// Is the current block terminated?
    pub fn is_current_block_terminated(&self) -> bool {
        self.basic_blocks[self.current_block_index].terminated
    }

    /// Number of basic blocks.
    pub fn basic_block_count(&self) -> usize {
        self.basic_blocks.len()
    }

    /// Is a specific block terminated?
    pub fn is_block_terminated(&self, index: usize) -> bool {
        self.basic_blocks[index].terminated
    }

    /// Dump all basic blocks and their instructions to a writer (for debugging).
    // --- Instruction emission ---

    /// Emit an instruction to the current basic block.
    pub fn emit(&mut self, instruction: Instruction) {
        if self.is_current_block_terminated() {
            return;
        }
        let source_map = SourceMapEntry {
            bytecode_offset: 0, // filled during flattening
            source_start: self.current_source_start,
            source_end: self.current_source_end,
        };
        let block = &mut self.basic_blocks[self.current_block_index];
        block.append(instruction, source_map);
    }

    /// Emit a Mov instruction (optimized away if src == dst).
    pub fn emit_mov(&mut self, dst: &ScopedOperand, src: &ScopedOperand) {
        if dst != src {
            self.emit(Instruction::Mov {
                dst: dst.operand(),
                src: src.operand(),
            });
        }
    }

    pub fn emit_mov_raw(&mut self, dst: Operand, src: Operand) {
        if dst != src {
            self.emit(Instruction::Mov { dst, src });
        }
    }

    // --- Cache index allocation ---

    pub fn next_property_lookup_cache(&mut self) -> u32 {
        let idx = self.next_property_lookup_cache;
        self.next_property_lookup_cache += 1;
        idx
    }

    pub fn next_global_variable_cache(&mut self) -> u32 {
        let idx = self.next_global_variable_cache;
        self.next_global_variable_cache += 1;
        idx
    }

    pub fn next_template_object_cache(&mut self) -> u32 {
        let idx = self.next_template_object_cache;
        self.next_template_object_cache += 1;
        idx
    }

    pub fn next_object_shape_cache(&mut self) -> u32 {
        let idx = self.next_object_shape_cache;
        self.next_object_shape_cache += 1;
        idx
    }

    // --- Boundary management ---

    pub fn start_boundary(&mut self, ty: BlockBoundaryType) {
        self.boundaries.push(ty);
    }

    pub fn end_boundary(&mut self, ty: BlockBoundaryType) {
        assert_eq!(self.boundaries.last(), Some(&ty));
        self.boundaries.pop();
    }

    // --- Break/continue scope management ---

    pub fn begin_breakable_scope(&mut self, target: Label, mut label_set: Vec<Vec<u16>>) {
        label_set.extend(self.pending_labels.iter().cloned());
        self.breakable_scopes.push(LabelableScope {
            bytecode_target: target,
            language_label_set: label_set,
            completion_register: None,
        });
        self.start_boundary(BlockBoundaryType::Break);
    }

    pub fn end_breakable_scope(&mut self) {
        self.end_boundary(BlockBoundaryType::Break);
        self.breakable_scopes.pop();
    }

    pub fn begin_continuable_scope(&mut self, target: Label, mut label_set: Vec<Vec<u16>>) {
        label_set.extend(self.pending_labels.iter().cloned());
        self.continuable_scopes.push(LabelableScope {
            bytecode_target: target,
            language_label_set: label_set,
            completion_register: None,
        });
        self.start_boundary(BlockBoundaryType::Continue);
    }

    pub fn end_continuable_scope(&mut self) {
        self.end_boundary(BlockBoundaryType::Continue);
        self.continuable_scopes.pop();
    }

    pub fn find_breakable_scope(&self, label: Option<&[u16]>) -> Option<&LabelableScope> {
        if let Some(label) = label {
            self.breakable_scopes
                .iter()
                .rev()
                .find(|s| s.language_label_set.iter().any(|l| l == label))
        } else {
            self.breakable_scopes.last()
        }
    }

    pub fn find_continuable_scope(&self, label: Option<&[u16]>) -> Option<&LabelableScope> {
        if let Some(label) = label {
            self.continuable_scopes
                .iter()
                .rev()
                .find(|s| s.language_label_set.iter().any(|l| l == label))
        } else {
            self.continuable_scopes.last()
        }
    }

    // --- FinallyContext support ---

    /// Check if there is an outer ReturnToFinally boundary between `boundary_index`
    /// and the matching break/continue boundary.
    fn has_outer_finally_before_target(&self, is_break: bool, boundary_index: usize) -> bool {
        for j in (0..boundary_index.saturating_sub(1)).rev() {
            let inner = self.boundaries[j];
            if (is_break && inner == BlockBoundaryType::Break)
                || (!is_break && inner == BlockBoundaryType::Continue)
            {
                return false;
            }
            if inner == BlockBoundaryType::ReturnToFinally {
                return true;
            }
        }
        false
    }

    /// Register a jump target with the current FinallyContext.
    /// Assigns a unique completion_type index and emits code to set it and jump to finally.
    pub fn register_jump_in_finally_context(&mut self, target: Label) {
        let ctx = self.current_finally_context.as_mut().unwrap();
        let jump_index = ctx.next_jump_index;
        ctx.next_jump_index += 1;
        ctx.registered_jumps.push(FinallyJump {
            index: jump_index,
            target,
        });
        let completion_type = ctx.completion_type.clone();
        let finally_body = ctx.finally_body;
        let idx_const = self.add_constant_i32(jump_index);
        self.emit_mov(&completion_type, &idx_const);
        self.emit(Instruction::Jump {
            target: finally_body,
        });
    }

    /// For break/continue through nested finally: create a trampoline block.
    fn emit_trampoline_through_finally(&mut self, is_break: bool) {
        let trampoline_block = self.make_block();
        self.register_jump_in_finally_context(Label(trampoline_block as u32));
        self.switch_to_basic_block(trampoline_block);
        // Pop to the parent FinallyContext (simulating the inner finally completing).
        let parent = self
            .current_finally_context
            .as_ref()
            .and_then(|c| c.parent.clone());
        self.current_finally_context = parent;
        // Continue the boundary walk from here (the caller will keep going).
        let _ = is_break; // Used by caller to determine direction
    }

    /// Generate a break, walking boundaries and handling FinallyContext.
    pub fn generate_break(&mut self, label: Option<&[u16]>) {
        if let Some(label) = label {
            self.generate_labelled_jump(true, label);
        } else {
            self.generate_scoped_jump(true);
        }
    }

    /// Generate a continue, walking boundaries and handling FinallyContext.
    pub fn generate_continue(&mut self, label: Option<&[u16]>) {
        if let Some(label) = label {
            self.generate_labelled_jump(false, label);
        } else {
            self.generate_scoped_jump(false);
        }
    }

    /// Walk boundaries for unlabelled break/continue.
    fn generate_scoped_jump(&mut self, is_break: bool) {
        let saved_finally_context = self.current_finally_context.clone();
        let env_stack_len = self.lexical_environment_register_stack.len();
        let mut env_offset = env_stack_len;

        let mut i = self.boundaries.len();
        while i > 0 {
            i -= 1;
            let boundary = self.boundaries[i];
            match boundary {
                BlockBoundaryType::Break if is_break => {
                    let target = self.breakable_scopes.last().unwrap().bytecode_target;
                    self.emit(Instruction::Jump { target });
                    self.current_finally_context = saved_finally_context;
                    return;
                }
                BlockBoundaryType::Continue if !is_break => {
                    let target = self.continuable_scopes.last().unwrap().bytecode_target;
                    self.emit(Instruction::Jump { target });
                    self.current_finally_context = saved_finally_context;
                    return;
                }
                BlockBoundaryType::LeaveLexicalEnvironment => {
                    env_offset -= 1;
                    let env = self.lexical_environment_register_stack[env_offset - 1].clone();
                    self.emit(Instruction::SetLexicalEnvironment {
                        environment: env.operand(),
                    });
                }
                BlockBoundaryType::ReturnToFinally => {
                    if !self.has_outer_finally_before_target(is_break, i + 1) {
                        let target = if is_break {
                            self.breakable_scopes.last().unwrap().bytecode_target
                        } else {
                            self.continuable_scopes.last().unwrap().bytecode_target
                        };
                        self.register_jump_in_finally_context(target);
                        self.current_finally_context = saved_finally_context;
                        return;
                    }
                    self.emit_trampoline_through_finally(is_break);
                }
                _ => {}
            }
        }
        self.current_finally_context = saved_finally_context;
    }

    /// Walk boundaries for labelled break/continue.
    fn generate_labelled_jump(&mut self, is_break: bool, label: &[u16]) {
        let saved_finally_context = self.current_finally_context.clone();
        let env_stack_len = self.lexical_environment_register_stack.len();
        let mut env_offset = env_stack_len;

        let jumpable_scopes: Vec<(Label, Vec<Vec<u16>>)> = if is_break {
            self.breakable_scopes
                .iter()
                .rev()
                .map(|s| (s.bytecode_target, s.language_label_set.clone()))
                .collect()
        } else {
            self.continuable_scopes
                .iter()
                .rev()
                .map(|s| (s.bytecode_target, s.language_label_set.clone()))
                .collect()
        };

        let mut current_boundary = self.boundaries.len();

        for (target, label_set) in &jumpable_scopes {
            while current_boundary > 0 {
                current_boundary -= 1;
                let boundary = self.boundaries[current_boundary];
                match boundary {
                    BlockBoundaryType::LeaveLexicalEnvironment => {
                        env_offset -= 1;
                        let env = self.lexical_environment_register_stack[env_offset - 1].clone();
                        self.emit(Instruction::SetLexicalEnvironment {
                            environment: env.operand(),
                        });
                    }
                    BlockBoundaryType::ReturnToFinally => {
                        if !self.has_outer_finally_before_target(is_break, current_boundary + 1)
                            && label_set.iter().any(|l| l == label)
                        {
                            self.register_jump_in_finally_context(*target);
                            self.current_finally_context = saved_finally_context;
                            return;
                        }
                        self.emit_trampoline_through_finally(is_break);
                    }
                    b if (is_break && b == BlockBoundaryType::Break)
                        || (!is_break && b == BlockBoundaryType::Continue) =>
                    {
                        break;
                    }
                    _ => {}
                }
            }

            if label_set.iter().any(|l| l == label) {
                self.emit(Instruction::Jump {
                    target: *target,
                });
                self.current_finally_context = saved_finally_context;
                return;
            }
        }
        self.current_finally_context = saved_finally_context;
    }

    /// Generate a return, routing through FinallyContext if needed.
    pub fn generate_return(&mut self, value: &ScopedOperand) {
        if let Some(ref ctx) = self.current_finally_context {
            let completion_value = ctx.completion_value.clone();
            let completion_type = ctx.completion_type.clone();
            let finally_body = ctx.finally_body;
            self.emit_mov(&completion_value, value);
            let ret_const = self.add_constant_i32(FinallyContext::RETURN);
            self.emit_mov(&completion_type, &ret_const);
            self.emit(Instruction::Jump {
                target: finally_body,
            });
        } else if self.is_in_generator_or_async_function() {
            self.emit(Instruction::Yield {
                continuation_label: None,
                value: value.operand(),
            });
        } else {
            self.emit(Instruction::Return {
                value: value.operand(),
            });
        }
    }

    pub fn add_constant_i32(&mut self, val: i32) -> ScopedOperand {
        self.add_constant_number(val as f64)
    }

    // --- Local variable initialization tracking ---

    pub fn is_local_initialized(&self, index: u32) -> bool {
        self.initialized_locals
            .get(index as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn mark_local_initialized(&mut self, index: u32) {
        let idx = index as usize;
        if idx >= self.initialized_locals.len() {
            self.initialized_locals.resize(idx + 1, false);
        }
        self.initialized_locals[idx] = true;
    }

    // --- Compile/assemble/link pipeline ---

    /// Compile all basic blocks into a flat bytecode buffer.
    ///
    /// This performs:
    /// 1. Operand rewriting (offset indices for the runtime layout)
    /// 2. Compute block byte offsets using encoded_size()
    /// 3. Patch labels in typed instructions (block index → byte offset)
    /// 4. Encode to bytes and build source map + exception handlers
    pub fn assemble(&mut self) -> AssembledBytecode {
        let number_of_registers = self.next_register;
        let number_of_locals = self.local_variables.len() as u32;
        let number_of_constants = self.constants.len() as u32;

        // Phase 1: Operand rewriting
        for block in &mut self.basic_blocks {
            for (inst, _) in &mut block.instructions {
                inst.visit_operands(&mut |op: &mut Operand| {
                    match op.operand_type() {
                        OperandType::Register => {} // stays as-is
                        OperandType::Local => op.offset_index_by(number_of_registers),
                        OperandType::Constant => {
                            op.offset_index_by(number_of_registers + number_of_locals)
                        }
                        OperandType::Argument => {
                            op.offset_index_by(
                                number_of_registers + number_of_locals + number_of_constants,
                            )
                        }
                    }
                });
            }
        }

        // Phase 2: Compute block byte offsets using encoded_size()
        let mut block_offsets: Vec<usize> = Vec::with_capacity(self.basic_blocks.len());
        let mut offset: usize = 0;
        for block in &self.basic_blocks {
            block_offsets.push(offset);
            for (inst, _) in &block.instructions {
                offset += inst.encoded_size();
            }
        }

        // Phase 3: Patch labels (block index → byte offset)
        for block in &mut self.basic_blocks {
            for (inst, _) in &mut block.instructions {
                inst.visit_labels(&mut |label: &mut Label| {
                    let block_index = label.0 as usize;
                    label.0 = block_offsets[block_index] as u32;
                });
            }
        }

        // Phase 4: Encode to bytes
        let mut bytecode: Vec<u8> = Vec::with_capacity(offset);
        let mut source_map: Vec<SourceMapEntry> = Vec::new();
        let mut exception_handlers: Vec<ExceptionHandler> = Vec::new();

        for (block_idx, block) in self.basic_blocks.iter().enumerate() {
            let block_start = bytecode.len();

            // Track exception handler range
            let handler = block.handler;

            for (inst, sm) in &block.instructions {
                let inst_offset = bytecode.len();

                source_map.push(SourceMapEntry {
                    bytecode_offset: inst_offset as u32,
                    source_start: sm.source_start,
                    source_end: sm.source_end,
                });

                inst.encode(self.strict, &mut bytecode);
            }

            // Close exception handler range
            if let Some(handler_block) = handler {
                exception_handlers.push(ExceptionHandler {
                    start_offset: block_start as u32,
                    end_offset: bytecode.len() as u32,
                    handler_offset: block_offsets[handler_block] as u32,
                });
            }

            let _ = block_idx;
        }

        self.finished = true;

        AssembledBytecode {
            bytecode,
            source_map,
            exception_handlers,
            basic_block_start_offsets: block_offsets,
            number_of_registers,
        }
    }
}

/// Result of assembling bytecode from basic blocks.
pub struct AssembledBytecode {
    pub bytecode: Vec<u8>,
    pub source_map: Vec<SourceMapEntry>,
    pub exception_handlers: Vec<ExceptionHandler>,
    pub basic_block_start_offsets: Vec<usize>,
    pub number_of_registers: u32,
}

/// Exception handler range (with byte offsets, post-linking).
#[derive(Debug, Clone)]
pub struct ExceptionHandler {
    pub start_offset: u32,
    pub end_offset: u32,
    pub handler_offset: u32,
}

/// A typed constant value stored in the constant pool.
///
/// The actual NaN-boxed encoding happens at the FFI boundary when
/// creating the C++ `Bytecode::Executable`.
#[derive(Debug, Clone)]
pub enum ConstantValue {
    Number(f64),
    Boolean(bool),
    Null,
    Undefined,
    Empty,
    String(Vec<u16>),
    BigInt(String),
}

/// Use `preferred_dst` if available, otherwise allocate a fresh register.
pub fn choose_dst(generator: &mut Generator, preferred_dst: Option<&ScopedOperand>) -> ScopedOperand {
    match preferred_dst {
        Some(dst) => dst.clone(),
        None => generator.allocate_register(),
    }
}

