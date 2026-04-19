//! LuMeLIR — Lua → MLIR → native AOT compiler toolchain.
//!
//! The library crate hosts the compiler pipeline layers:
//!
//! ```text
//! cli → codegen → mir → hir → parser → lexer
//! ```
//!
//! Each layer may only depend on layers strictly inside it. Side effects
//! (I/O, process spawn) are confined to [`cli`]; inner layers are pure.
//!
//! See `docs/design/0002-lib-rs-layering.md` for the full layering rationale.

pub mod cli;
pub mod lexer;
