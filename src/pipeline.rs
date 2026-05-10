//! Compile-pipeline use-case layer.
//!
//! Provides a stop-able pipeline that runs from Lua source through
//! HIR / MLIR / LLVM IR, halting at a named [`EmitStage`] and
//! returning the corresponding [`PipelineArtifact`]. Used by the
//! CLI `--emit` flag (ADR 0090) and reserved for future
//! programmatic / DAP / LSP consumers that need to inspect
//! intermediate artifacts without spawning the full native-binary
//! pipeline.
//!
//! Effect boundary (ADR 0090):
//!
//! - [`PipelineArtifact::Hir`] is **rendered** purely
//!   (`format!("{:#?}", hir)`).
//! - [`PipelineArtifact::Mlir`] is **rendered** purely
//!   (`module.as_operation().to_string()` — borrows the MLIR
//!   `Context` but performs no external I/O).
//! - [`PipelineArtifact::Llvm`] is **generated** with side effects:
//!   internally invokes `mlir-opt` and `mlir-translate` as
//!   subprocesses via the existing
//!   [`crate::codegen::lower::to_llvm_ir`] helper.
//!
//! See `docs/design/0090-phase2-devinfra-emit.md` for the full
//! rationale and the Codex review v2 that drove the
//! [`EmitStage`] / [`PipelineArtifact`] split out of the CLI layer.

use anyhow::{Context, Result};
use melior::ir::Module;

use crate::codegen::emit::{emit_module, new_context};
use crate::codegen::lower::to_llvm_ir;
use crate::hir::{HirChunk, lower};
use crate::parser::parse;

/// Pipeline halt point. Each variant names a textual artifact that
/// the pipeline can produce on the way to a native binary.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EmitStage {
    /// HIR after [`crate::hir::lower`]; pure render via Debug.
    Hir,
    /// MLIR module after [`crate::codegen::emit::emit_module`]; pure
    /// render via `module.as_operation().to_string()`.
    Mlir,
    /// LLVM IR after `mlir-opt` + `mlir-translate`; effectful
    /// (subprocess), via [`crate::codegen::lower::to_llvm_ir`].
    Llvm,
}

/// Textual artifact produced by [`compile_until`]. Variant matches
/// the [`EmitStage`] requested.
#[derive(Debug)]
pub enum PipelineArtifact {
    Hir(String),
    Mlir(String),
    Llvm(String),
}

impl PipelineArtifact {
    /// Borrow the inner text without distinguishing variants — used
    /// by I/O adapters that only need the rendered string.
    pub fn as_text(&self) -> &str {
        match self {
            PipelineArtifact::Hir(s) | PipelineArtifact::Mlir(s) | PipelineArtifact::Llvm(s) => s,
        }
    }
}

/// Run the pipeline up to `stage` and return its textual artifact.
/// Errors from any underlying layer (parser / HIR / codegen / external
/// MLIR-LLVM tools) are wrapped via `anyhow::Context` for CLI
/// diagnostics.
pub fn compile_until(source: &str, stage: EmitStage) -> Result<PipelineArtifact> {
    let chunk = parse(source)
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("parse")?;
    let hir = lower(&chunk)
        .map_err(|e| anyhow::anyhow!("{e:?}"))
        .context("hir lower")?;
    if matches!(stage, EmitStage::Hir) {
        return Ok(PipelineArtifact::Hir(render_hir(&hir)));
    }
    let context = new_context();
    let module = emit_module(&context, &hir)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("emit_module")?;
    if matches!(stage, EmitStage::Mlir) {
        return Ok(PipelineArtifact::Mlir(render_mlir(&module)));
    }
    let llvm_ir = generate_llvm_ir(&module).context("generate llvm ir")?;
    Ok(PipelineArtifact::Llvm(llvm_ir))
}

/// Pure: HIR Debug rendering. No I/O, no subprocess.
fn render_hir(hir: &HirChunk) -> String {
    format!("{hir:#?}")
}

/// Pure: MLIR module text rendering. No external subprocess; only
/// MLIR `Context`-internal traversal.
fn render_mlir(module: &Module<'_>) -> String {
    module.as_operation().to_string()
}

/// Effectful: invokes `mlir-opt` + `mlir-translate` subprocesses via
/// the existing [`crate::codegen::lower::to_llvm_ir`] helper.
/// Failures from external tools surface as `anyhow::Error`.
fn generate_llvm_ir(module: &Module<'_>) -> Result<String> {
    to_llvm_ir(module).map_err(|e| anyhow::anyhow!("{e}"))
}
