//! MLIR code generation from parsed AST.
//!
//! Pipeline:
//! ```text
//! Expr (AST) → emit_module() → MLIR Module
//!            → to_llvm_ir()  → LLVM IR text
//!            → to_native()   → native binary
//! ```
//!
//! Phase 1 uses standard MLIR dialects only (`arith`, `func`, `llvm`).
//! See `docs/design/0006-phase1-codegen.md` for design rationale.

pub mod emit;
pub mod error;
pub mod link;
pub mod lower;

use std::path::Path;

use crate::parser::Expr;
use emit::{emit_module, new_context};
use error::CodegenError;
use link::to_native;
use lower::to_llvm_ir;

/// Compile an AST expression to a native binary at `output`.
pub fn compile(expr: &Expr, output: &Path) -> Result<(), CodegenError> {
    let context = new_context();
    let module = emit_module(&context, expr)?;
    let llvm_ir = to_llvm_ir(&module)?;
    to_native(&llvm_ir, output)?;
    Ok(())
}
