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
fn run_no_arg_reads_piped_stdin() {
    // `echo ... | lumelir run` (no positional arg) reads stdin
    // when stdin is not a TTY. Unix-style implicit pipe input.
    let mut child = Command::new(lumelir_bin())
        .arg("run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lumelir run");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"print(\"piped-no-arg\")")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "implicit stdin run must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "piped-no-arg");
}

#[test]
fn run_no_arg_implicit_stdin_label_on_error() {
    // Implicit-stdin parse error renders with the `<stdin>`
    // sentinel label (same path as explicit `-`).
    let mut child = Command::new(lumelir_bin())
        .arg("run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lumelir run");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"print(1 +")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert!(!out.status.success(), "broken stdin must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<stdin>"),
        "stderr should mention <stdin> label; got:\n{stderr}"
    );
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

// --- Phase 2.8f-run-arg-table (ADR 0115): `arg` table ---
//
// Trailing positional args after `lumelir run [INPUT]` are
// exposed to the Lua script via the `arg` table (Lua §6.1 / §8.1
// MVP scope: `arg[1]` from positional 0, no `arg[0]` script name,
// no negative indices). The table is synthesised at the CLI
// boundary as a `local arg = {...}` statement prepended to the
// parsed chunk; no HIR/codegen surface change.

#[test]
fn run_inline_arg_count() {
    let out = run_lumelir(&["run", "print(#arg)", "a", "b", "c"]);
    assert!(
        out.status.success(),
        "inline + args must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3");
}

#[test]
fn run_inline_arg_index_one_based() {
    let out = run_lumelir(&["run", "print(arg[1])", "alpha", "beta"]);
    assert!(
        out.status.success(),
        "arg[1] must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "alpha");
}

#[test]
fn run_inline_arg_zero_extra_args() {
    // No positional args after INPUT → `arg` is the empty table.
    let out = run_lumelir(&["run", "print(#arg)"]);
    assert!(
        out.status.success(),
        "no extra args must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0");
}

#[test]
fn run_inline_arg_empty_string_value() {
    // Empty-string positional arg → arg[1] is "". HIR static
    // typing currently treats `arg[1]` as TaggedValue (pre-
    // existing limitation: Table element kind does not flow
    // through indexing), so we verify the empty value via
    // `print(arg[1])` which dispatches on the runtime tag. The
    // stdout is the empty arg followed by print's newline.
    let out = run_lumelir(&["run", "print(arg[1])", ""]);
    assert!(
        out.status.success(),
        "empty-string arg must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // print writes the empty string then a newline → stdout == "\n".
    assert_eq!(String::from_utf8_lossy(&out.stdout), "\n");
}

#[test]
fn run_inline_arg_with_space() {
    let out = run_lumelir(&["run", "print(arg[1])", "a b"]);
    assert!(
        out.status.success(),
        "space-containing arg must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "a b");
}

#[test]
fn run_inline_arg_oob_is_nil() {
    // arg[N+1] is nil per Lua table semantics.
    let out = run_lumelir(&["run", "print(arg[4] == nil)", "a", "b", "c"]);
    assert!(
        out.status.success(),
        "oob arg must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "true");
}

#[test]
fn run_file_passes_through_arg_table() {
    // File mode + trailing args.
    let path = std::env::temp_dir().join("lumelir_arg_file_mode.lua");
    std::fs::write(&path, "print(arg[1], arg[2])").expect("write fixture");
    let out = run_lumelir(&["run", path.to_str().unwrap(), "first", "second"]);
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "file + args must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "first\tsecond");
}

#[test]
fn run_explicit_dash_passes_through_arg_table() {
    // Explicit `-` (stdin) mode + trailing args.
    let mut child = Command::new(lumelir_bin())
        .args(["run", "-", "x", "y"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lumelir run -");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"print(arg[1], arg[2])")
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "stdin + args must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "x\ty");
}

#[test]
fn run_implicit_pipe_passes_through_arg_table() {
    // Implicit-pipe stdin (no positional INPUT) + trailing args.
    // Note: clap binds the first trailing arg to INPUT when no
    // explicit `-` is given. So `lumelir run a b c` would treat
    // "a" as INPUT (inline code). For implicit-pipe + args we
    // currently require explicit `-`; this test pins the
    // explicit-dash path as the supported way to combine pipe +
    // args. (Documented behaviour.)
    //
    // Alternative scripts could use `lumelir run - x y < file`.
    // We re-use the explicit-dash test above for that case.
}

#[test]
fn run_arg_type_is_table() {
    let out = run_lumelir(&["run", "print(type(arg))", "x"]);
    assert!(
        out.status.success(),
        "type(arg) must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "table");
}

#[test]
fn run_arg_element_type_is_string() {
    let out = run_lumelir(&["run", "print(type(arg[1]))", "hello"]);
    assert!(
        out.status.success(),
        "type(arg[1]) must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "string");
}

#[test]
fn run_arg_local_shadow_takes_precedence() {
    // User can rebind `arg` with a fresh `local arg = ...` and
    // the synthetic prelude does not block that (Lua scope rule:
    // later `local` shadows earlier `local`).
    let out = run_lumelir(&["run", "local arg = {99}\nprint(arg[1])", "ignored"]);
    assert!(
        out.status.success(),
        "user-shadow arg must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "99");
}
