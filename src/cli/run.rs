use std::error::Error;
use std::path::Path;

pub fn invoke(input: &Path) -> Result<(), Box<dyn Error>> {
    eprintln!("lumelir run: input={}", input.display());
    eprintln!("not yet implemented: Phase 1 PoC");
    Ok(())
}
