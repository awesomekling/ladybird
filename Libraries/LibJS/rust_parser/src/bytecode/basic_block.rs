/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::instruction::Instruction;

/// A source map entry mapping a bytecode offset to a source range.
#[derive(Debug, Clone, Copy)]
pub struct SourceMapEntry {
    pub bytecode_offset: u32,
    pub source_start: u32,
    pub source_end: u32,
}

/// A basic block in the bytecode generator.
///
/// During codegen, instructions are appended as typed `Instruction` enum
/// variants. During flattening (compile/assemble), instructions are
/// serialized into the final byte stream.
pub struct BasicBlock {
    pub index: u32,
    pub instructions: Vec<(Instruction, SourceMapEntry)>,
    pub handler: Option<usize>,
    pub terminated: bool,
}

impl BasicBlock {
    pub fn new(index: u32) -> Self {
        Self {
            index,
            instructions: Vec::new(),
            handler: None,
            terminated: false,
        }
    }

    pub fn append(&mut self, instruction: Instruction, source_map: SourceMapEntry) {
        let is_terminator = instruction.is_terminator();
        self.instructions.push((instruction, source_map));
        if is_terminator {
            self.terminated = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }
}
