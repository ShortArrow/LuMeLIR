//! ADR 0315 — N5-A: `coroutine.create` value + runtime foundation.
//!
//! darwin is bake-gated (ADR 0315 §2): ucontext on macOS is
//! unverified; these tests compile only on non-macOS hosts.
#![cfg(not(target_os = "macos"))]

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
fn create_returns_thread_type() {
    let out = run_ok(
        r#"
local co = coroutine.create(function() end)
print(type(co))
"#,
        "lumelir_coro_create_type",
    );
    assert_eq!(out.trim(), "thread");
}

#[test]
fn create_value_identity_eq() {
    let out = run_ok(
        r#"
local a = coroutine.create(function() end)
local b = coroutine.create(function() end)
print(a == a)
print(a == b)
"#,
        "lumelir_coro_create_eq",
    );
    assert_eq!(out.trim(), "true\nfalse");
}

/// Hash-keyed store: the array path (`t[1] = ...`) rejects
/// TaggedValue values for ALL tagged sources (ADR 0138-M, applies
/// equally to `newproxy()`) — an inherited boundary, not a
/// coroutine gap (ADR 0315).
#[test]
fn create_value_stores_in_table() {
    let out = run_ok(
        r#"
local t = {}
t.co = coroutine.create(function() end)
print(type(t.co))
"#,
        "lumelir_coro_create_tbl",
    );
    assert_eq!(out.trim(), "thread");
}
