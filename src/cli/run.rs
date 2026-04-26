use std::path::Path;

use anyhow::{Context, Result};

pub fn invoke(input: &Path) -> Result<()> {
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;

    let expr = crate::parser::parse(&source).map_err(|e| anyhow::anyhow!("parse error: {e}"))?;

    let tmp = std::env::temp_dir().join("lumelir_run_tmp");

    crate::codegen::compile(&expr, &tmp).map_err(|e| anyhow::anyhow!("codegen error: {e}"))?;

    let status = std::process::Command::new(&tmp)
        .status()
        .with_context(|| format!("executing {}", tmp.display()))?;

    let _ = std::fs::remove_file(&tmp);

    if !status.success() {
        anyhow::bail!("program exited with {status}");
    }
    Ok(())
}
