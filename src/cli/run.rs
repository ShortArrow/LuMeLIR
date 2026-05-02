use std::path::Path;

use anyhow::{Context, Result};

use super::diag;

pub fn invoke(input: &Path) -> Result<()> {
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;

    let chunk = crate::parser::parse(&source)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, input, "parse", &e)))?;
    let hir = crate::hir::lower(&chunk)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, input, "hir", &e)))?;

    let tmp = std::env::temp_dir().join("lumelir_run_tmp");

    crate::codegen::compile(&hir, &tmp).map_err(|e| anyhow::anyhow!("codegen error: {e}"))?;

    let status = std::process::Command::new(&tmp)
        .status()
        .with_context(|| format!("executing {}", tmp.display()))?;

    let _ = std::fs::remove_file(&tmp);

    if !status.success() {
        anyhow::bail!("program exited with {status}");
    }
    Ok(())
}
