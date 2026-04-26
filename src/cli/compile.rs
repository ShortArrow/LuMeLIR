use std::path::Path;

use anyhow::{Context, Result};

pub fn invoke(input: &Path, output: Option<&Path>, _target: &str) -> Result<()> {
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;

    let chunk = crate::parser::parse(&source).map_err(|e| anyhow::anyhow!("parse error: {e}"))?;
    let hir = crate::hir::lower(&chunk).map_err(|e| anyhow::anyhow!("hir error: {e}"))?;

    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => input.with_extension(""),
    };

    crate::codegen::compile(&hir, &output_path)
        .map_err(|e| anyhow::anyhow!("codegen error: {e}"))?;

    eprintln!("compiled → {}", output_path.display());
    Ok(())
}
