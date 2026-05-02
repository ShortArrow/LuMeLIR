//! Integration test: Phase 2.9a — line/column diagnostics
//! (ADR 0045). Lower layers carry byte offsets; the CLI boundary
//! renders them as `path:line:col: <layer> error: <message>`.

use std::process::Command;

fn run_compile(source: &str, name: &str) -> (bool, String) {
    let dir = std::env::temp_dir();
    let lua_path = dir.join(format!("{name}.lua"));
    std::fs::write(&lua_path, source).expect("write source");
    let out = Command::new(env!("CARGO_BIN_EXE_lumelir"))
        .args([
            "compile",
            lua_path.to_str().unwrap(),
            "-o",
            dir.join(format!("{name}.bin")).to_str().unwrap(),
        ])
        .output()
        .expect("invoke lumelir compile");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_file(&lua_path);
    (out.status.success(), stderr)
}

#[test]
fn hir_undefined_name_renders_with_line_col() {
    // "z" is undefined. With "local x = 1\nlocal y = z\n" the
    // offset of `z` is 22 → line 2, col 11 (1-based).
    let (ok, stderr) = run_compile("local x = 1\nlocal y = z\n", "lumelir_29a_hir");
    assert!(!ok);
    assert!(
        stderr.contains(":2:11:"),
        "expected line:col 2:11 in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("hir error:"),
        "expected hir error layer tag, got: {stderr}"
    );
    assert!(
        stderr.contains("undefined name 'z'"),
        "expected the original message, got: {stderr}"
    );
}

#[test]
fn parse_error_renders_with_line_col() {
    // `local x =` with nothing after produces an UnexpectedEof at
    // some byte after the `=`.
    let (ok, stderr) = run_compile("local x =\n", "lumelir_29a_parse");
    assert!(!ok);
    assert!(
        stderr.contains("parse error:"),
        "expected parse error tag, got: {stderr}"
    );
    // The error should reference some line/col, not a bare "byte offset".
    assert!(
        !stderr.contains("byte offset 0\n"),
        "raw byte offset should be replaced, got: {stderr}"
    );
}

#[test]
fn lex_error_renders_with_line_col() {
    // `\q` is an unrecognised escape — produces a LexError.
    let (ok, stderr) = run_compile("print(\"a\\qb\")\n", "lumelir_29a_lex");
    assert!(!ok);
    assert!(
        stderr.contains("parse error:"),
        "expected parse error wrapper around the lex error, got: {stderr}"
    );
    // The escape position is on line 1.
    assert!(
        stderr.contains(":1:"),
        "expected line 1 prefix, got: {stderr}"
    );
}
