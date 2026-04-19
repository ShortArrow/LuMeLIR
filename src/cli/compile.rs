use std::path::Path;

use anyhow::Result;

pub fn invoke(input: &Path, output: Option<&Path>, target: &str) -> Result<()> {
    eprintln!(
        "lumelir compile: input={} output={} target={}",
        input.display(),
        output
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<auto>".into()),
        target,
    );
    eprintln!("not yet implemented: Phase 1 PoC");
    Ok(())
}
