use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::diag;

/// Resolve `lumelir run <INPUT>` into `(source, label_path)`.
///
/// Three modes:
/// - `-` → read stdin, label `<stdin>`.
/// - `<input>` names an existing file → read it, label = the path.
/// - Otherwise treat `<input>` as inline Lua code, label `<inline>`.
///
/// The label is used purely by `diag::format_error` for the
/// "file:line:col" prefix on parse/HIR errors. Inline / stdin
/// modes synthesise a sentinel path so the diagnostic format stays
/// uniform.
fn resolve_input(input: &str) -> Result<(String, PathBuf)> {
    if input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        return Ok((buf, PathBuf::from("<stdin>")));
    }
    let path = Path::new(input);
    if path.is_file() {
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        return Ok((source, path.to_path_buf()));
    }
    // Inline Lua source.
    Ok((input.to_owned(), PathBuf::from("<inline>")))
}

pub fn invoke(input: &str) -> Result<()> {
    let (source, label) = resolve_input(input)?;

    let chunk = crate::parser::parse(&source)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, &label, "parse", &e)))?;
    let hir = crate::hir::lower(&chunk)
        .map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, &label, "hir", &e)))?;

    // Per-process scratch path so concurrent `lumelir run`
    // invocations (e.g. parallel cargo tests) do not race on the
    // same artifact file.
    let tmp = std::env::temp_dir().join(format!("lumelir_run_tmp_{}", std::process::id()));

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
