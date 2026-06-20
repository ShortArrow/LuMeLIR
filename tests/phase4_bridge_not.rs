//! ADR 0225 — Rust-Lua Bridge Bool ↔ Bool marshaling demo.
//! `rust.invert(b)` dispatches to `extern "C" fn rust_not(bool) ->
//! bool` in `src/bridge_runtime.rs`. The codegen widens i1 to
//! i8 at the call boundary to match the C ABI and truncates
//! back to i1 on the return.

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
fn rust_not_true_returns_false() {
    assert_eq!(
        run_ok("print(rust.invert(true))", "lumelir_bridge_not_true").trim(),
        "false"
    );
}

#[test]
fn rust_not_false_returns_true() {
    assert_eq!(
        run_ok("print(rust.invert(false))", "lumelir_bridge_not_false").trim(),
        "true"
    );
}

#[test]
fn rust_not_round_trip_is_identity() {
    // Double-NOT round-trip: not(not(x)) == x. Proves the i1→i8→
    // i1 ABI shape preserves the Bool value across two calls.
    let out = run_ok(
        "print(rust.invert(rust.invert(true)))\nprint(rust.invert(rust.invert(false)))",
        "lumelir_bridge_not_double",
    );
    assert_eq!(out.trim(), "true\nfalse");
}

#[test]
fn rust_not_feeds_into_if() {
    // The returned Bool flows into a Lua `if` predicate the same
    // as any literal `true` / `false` — proves the codegen
    // produces a kind-correct i1 result.
    let out = run_ok(
        "if rust.invert(false) then print(\"taken\") else print(\"missed\") end",
        "lumelir_bridge_not_in_if",
    );
    assert_eq!(out.trim(), "taken");
}
