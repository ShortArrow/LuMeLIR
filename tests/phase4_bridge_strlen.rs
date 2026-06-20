//! ADR 0224 — Rust-Lua Bridge String → Number marshaling demo.
//! `rust.strlen(s)` dispatches to `extern "C" fn rust_strlen` in
//! `src/bridge_runtime.rs`, which reads the i64 length header
//! at offset 0 of the boxed-string-object pointer (ADR 0112
//! layout) and returns it as f64 (Lua Number ABI).

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn rust_strlen_basic() {
    assert_eq!(
        run_ok(
            "print(rust.strlen(\"hello\"))",
            "lumelir_bridge_strlen_basic"
        )
        .trim(),
        "5"
    );
}

#[test]
fn rust_strlen_empty_string_returns_zero() {
    assert_eq!(
        run_ok("print(rust.strlen(\"\"))", "lumelir_bridge_strlen_empty").trim(),
        "0"
    );
}

#[test]
fn rust_strlen_matches_lua_string_len() {
    // Cross-check: the Rust bridge reads the same i64 header as
    // the existing `string.len` builtin (ADR 0103).
    assert_eq!(
        run_ok(
            "local s = \"lua-style\"\nprint(rust.strlen(s))\nprint(string.len(s))",
            "lumelir_bridge_strlen_xcheck"
        )
        .trim(),
        "9\n9"
    );
}

#[test]
fn rust_strlen_handles_concat_string() {
    // The result of `..` concat is a heap-allocated boxed string
    // with the same ADR 0112 layout; rust_strlen reads it without
    // any allocation hand-off.
    assert_eq!(
        run_ok(
            "local s = \"abc\" .. \"defgh\"\nprint(rust.strlen(s))",
            "lumelir_bridge_strlen_concat"
        )
        .trim(),
        "8"
    );
}
