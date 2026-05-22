//! Phase 2.devinfra-run-modes: `lumelir run` accepts file path,
//! inline Lua code, or stdin (`-`).
//!
//! Selection rule (CLI-resolved at `src/cli/run.rs::resolve_input`):
//! 1. `INPUT == "-"` → read stdin.
//! 2. `INPUT` names an existing file → load as source, error label
//!    = the path.
//! 3. Otherwise treat `INPUT` as inline Lua source, error label
//!    = `<inline>`.
//!
//! Error labels are the sole observable difference between the
//! file-mode and inline-mode error paths; tests pin each one.

use std::io::Write;
use std::process::{Command, Stdio};

fn lumelir_bin() -> &'static str {
    env!("CARGO_BIN_EXE_lumelir")
}

fn run_lumelir(args: &[&str]) -> std::process::Output {
    Command::new(lumelir_bin())
        .args(args)
        .output()
        .expect("failed to run lumelir CLI")
}

#[test]
fn run_inline_executes_lua_source() {
    // Positional arg containing valid Lua source is interpreted
    // as inline code when it does not name an existing file.
    let out = run_lumelir(&["run", "print(1 + 2)"]);
    assert!(
        out.status.success(),
        "inline run must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "3");
}

#[test]
fn run_file_path_still_works() {
    // Existing-file path takes precedence over the inline fallback.
    let path = std::env::temp_dir().join("lumelir_run_modes_file.lua");
    std::fs::write(&path, "print(11 * 7)").expect("write fixture");
    let out = run_lumelir(&["run", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "file run must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "77");
}

#[test]
fn run_stdin_dash_reads_source() {
    // `lumelir run -` reads source from stdin.
    let mut child = Command::new(lumelir_bin())
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lumelir run -");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"print(\"stdin-mode\")")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stdin run must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "stdin-mode");
}

#[test]
fn run_inline_syntax_error_labels_with_inline_sentinel() {
    // Parse error from inline source uses `<inline>` as the label
    // in the diagnostic prefix, so the user can tell it was not
    // read from a real file.
    let out = run_lumelir(&["run", "print(1 +"]);
    assert!(!out.status.success(), "broken inline must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<inline>"),
        "stderr should mention <inline> label; got:\n{stderr}"
    );
    assert!(
        stderr.contains("parse error"),
        "stderr should report parse error; got:\n{stderr}"
    );
}
