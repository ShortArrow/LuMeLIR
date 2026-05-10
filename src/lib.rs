//! LuMeLIR — Lua → MLIR → native AOT compiler toolchain.
//!
//! The library crate hosts the compiler pipeline layers:
//!
//! ```text
//! cli → pipeline → codegen → mir → hir → parser → lexer
//! ```
//!
//! Each layer may only depend on layers strictly inside it. Side effects
//! (I/O, process spawn) are confined to [`cli`]; inner pure-compile-time
//! layers (parser, HIR transforms, MLIR construction) stay pure. ADR 0090
//! adds [`pipeline`] as a thin use-case orchestrator that aggregates the
//! existing per-stage helpers (`hir::lower`, `codegen::emit::emit_module`,
//! `codegen::lower::to_llvm_ir`) so the CLI's `--emit` flag can stop the
//! pipeline at a named stage; the subprocess-spawning behaviour of
//! `to_llvm_ir` (mlir-opt / mlir-translate) is unchanged and confined
//! to its existing call site.
//!
//! See `docs/design/0002-lib-rs-layering.md` for the full layering rationale.

pub mod cli;
pub mod codegen;
pub mod hir;
pub mod lexer;
pub mod parser;
pub mod pipeline;
