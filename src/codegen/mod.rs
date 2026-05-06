//! MLIR code generation from a resolved HIR chunk.
//!
//! Pipeline:
//! ```text
//! HirChunk → emit_module() → MLIR Module
//!          → to_llvm_ir()  → LLVM IR text
//!          → to_native()   → native binary
//! ```
//!
//! Standard MLIR dialects only (`arith`, `func`, `llvm`). See ADRs 0006
//! and 0007 for the Phase 1 baseline and Phase 2.0 extensions.

pub(crate) mod callabi;
pub mod emit;
pub mod error;
pub mod link;
pub mod lower;
pub(crate) mod primitive;
pub(crate) mod tagged;

use std::path::Path;

use crate::hir::HirChunk;
use emit::{emit_module, new_context};
use error::CodegenError;
use link::to_native;
use lower::to_llvm_ir;

/// Compile a resolved HIR chunk to a native binary at `output`.
pub fn compile(chunk: &HirChunk, output: &Path) -> Result<(), CodegenError> {
    let context = new_context();
    let module = emit_module(&context, chunk)?;
    let llvm_ir = to_llvm_ir(&module)?;
    to_native(&llvm_ir, output)?;
    Ok(())
}
