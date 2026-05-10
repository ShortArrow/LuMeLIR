use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::pipeline::{EmitStage, compile_until};

use super::diag;

pub fn invoke(
    input: &Path,
    output: Option<&Path>,
    _target: &str,
    emit: Option<EmitStage>,
) -> Result<()> {
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;

    if let Some(stage) = emit {
        // ADR 0090: pipeline-stage emission. Delegate to the
        // pipeline use-case for the parse / lower / emit_module /
        // generate_llvm_ir orchestration; CLI handles only the
        // output-destination choice (stdout default; -o file).
        // Errors propagate via anyhow::Error from the pipeline
        // layer; richer line/col diagnostics (`diag::format_error`)
        // remain on the no-emit path for now — a future devinfra
        // ADR can plumb HasOffset through `compile_until`.
        let artifact = compile_until(&source, stage)?;
        return write_dump(artifact.as_text(), output);
    }

    let chunk = crate::parser::parse(&source)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, input, "parse", &e)))?;
    let hir = crate::hir::lower(&chunk)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, input, "hir", &e)))?;

    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => input.with_extension(""),
    };

    crate::codegen::compile(&hir, &output_path)
        .map_err(|e| anyhow::anyhow!("codegen error: {e}"))?;

    eprintln!("compiled → {}", output_path.display());
    Ok(())
}

/// ADR 0090: I/O adapter for `--emit`. stdout default; `-o PATH`
/// writes to file. Status messages (if any) go to stderr — this
/// helper is silent on stdout when `-o` is set so the user can
/// pipe `--emit llvm > app.ll` cleanly.
fn write_dump(text: &str, output: Option<&Path>) -> Result<()> {
    match output {
        Some(path) => {
            std::fs::write(path, text)
                .with_context(|| format!("writing dump to {}", path.display()))?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle
                .write_all(text.as_bytes())
                .context("writing dump to stdout")?;
        }
    }
    Ok(())
}
