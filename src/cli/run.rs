use std::path::Path;

use anyhow::Result;

pub fn invoke(input: &Path) -> Result<()> {
    eprintln!("lumelir run: input={}", input.display());
    eprintln!("not yet implemented: Phase 1 PoC");
    Ok(())
}
