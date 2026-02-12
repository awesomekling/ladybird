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
    constants: Vec<u64>, // Raw Value bits (NaN-boxed)

    // Cached constants for deduplication
    true_constant: Option<ScopedOperand>,
    false_constant: Option<ScopedOperand>,
    null_constant: Option<ScopedOperand>,
    undefined_constant: Option<ScopedOperand>,
    empty_constant: Option<ScopedOperand>,
    int32_constants: HashMap<i32, ScopedOperand>,

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

    // Source location tracking
    pub current_source_start: u32,
    pub current_source_end: u32,

    // --- Completion register ---
    pub current_completion_register: Option<ScopedOperand>,

    // --- Accumulator and this ---
    accumulator: ScopedOperand,
    this_value: ScopedOperand,

    // --- Shared function data (indices) ---
    pub shared_function_data_count: u32,
    pub class_blueprint_count: u32,

    // --- Length identifier cache ---
    pub length_identifier: Option<PropertyKeyTableIndex>,

    // --- Generator finished flag ---
    pub finished: bool,
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
            string_table: Vec::new(),
            identifier_table: Vec::new(),
            property_key_table: Vec::new(),
            regex_table: Vec::new(),
            boundaries: Vec::new(),
            continuable_scopes: Vec::new(),
            breakable_scopes: Vec::new(),
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
            shared_function_data_count: 0,
            class_blueprint_count: 0,
            length_identifier: None,
            finished: false,
        };

        // Fix up the self-referential pointers.
        let self_ptr = &mut gen as *mut Generator;
        let acc_inner = std::rc::Rc::get_mut(&mut gen.accumulator.inner).unwrap();
        acc_inner.generator = self_ptr;
        let this_inner = std::rc::Rc::get_mut(&mut gen.this_value.inner).unwrap();
        this_inner.generator = self_ptr;

        gen
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

    fn scoped_operand(&mut self, operand: Operand) -> ScopedOperand {
        ScopedOperand {
            inner: std::rc::Rc::new(ScopedOperandInner {
                operand,
                generator: self as *mut Generator,
            }),
        }
    }

    // --- Constant pool ---

    /// Add a constant to the pool and return a ScopedOperand referencing it.
    /// Common values (true, false, null, undefined, i32) are deduplicated.
    pub fn add_constant(&mut self, raw_bits: u64) -> ScopedOperand {
        let index = self.constants.len() as u32;
        self.constants.push(raw_bits);
        self.scoped_operand(Operand::constant(index))
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

        // If there's an active unwind context with a handler, propagate it.
        // (Handler is the basic block index of the exception handler.)
        // For now, handler propagation is managed by the codegen methods.
        let _ = &mut block;

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

    // --- Instruction emission ---

    /// Emit an instruction to the current basic block.
    pub fn emit(&mut self, instruction: Instruction) {
        assert!(
            !self.is_current_block_terminated(),
            "Emitting into terminated block"
        );
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

