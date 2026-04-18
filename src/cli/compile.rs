use std::error::Error;
use std::path::Path;

pub fn invoke(
    input: &Path,
    output: Option<&Path>,
    target: &str,
) -> Result<(), Box<dyn Error>> {
    eprintln!(
        "lumelir compile: input={} output={} target={}",
        input.display(),
        output.map(|p| p.display().to_string()).unwrap_or_else(|| "<auto>".into()),
        target,
    );
    eprintln!("not yet implemented: Phase 1 PoC");
    Ok(())
}
