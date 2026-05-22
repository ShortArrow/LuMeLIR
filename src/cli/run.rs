use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::diag;

/// Resolve `lumelir run [INPUT]` into `(source, label_path)`.
///
/// Four modes:
/// - `INPUT` absent → if stdin is piped (not a TTY) read stdin
///   with label `<stdin>`; if stdin is a TTY error out with a
///   usage hint (no implicit REPL today).
/// - `INPUT == "-"` → read stdin explicitly, label `<stdin>`.
/// - `INPUT` names an existing file → read it, label = the path.
/// - Otherwise treat `INPUT` as inline Lua code, label `<inline>`.
///
/// The label is used purely by `diag::format_error` for the
/// "file:line:col" prefix on parse/HIR errors. Inline / stdin
/// modes synthesise a sentinel path so the diagnostic format stays
/// uniform.
fn read_stdin_source() -> Result<(String, PathBuf)> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok((buf, PathBuf::from("<stdin>")))
}

fn resolve_input(input: Option<&str>) -> Result<(String, PathBuf)> {
    let Some(arg) = input else {
        if std::io::stdin().is_terminal() {
            anyhow::bail!(
                "no input provided. Pass a file path, inline Lua code (e.g. `lumelir \
                 run 'print(1)'`), `-` to read stdin explicitly, or pipe source into \
                 stdin."
            );
        }
        return read_stdin_source();
    };
    if arg == "-" {
        return read_stdin_source();
    }
    let path = Path::new(arg);
    if path.is_file() {
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        return Ok((source, path.to_path_buf()));
    }
    // Inline Lua source.
    Ok((arg.to_owned(), PathBuf::from("<inline>")))
}

pub fn invoke(input: Option<&str>) -> Result<()> {
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
