//! ADR 0226 — Rust-Lua Bridge mixed-kind multi-arg marshaling.
//! `rust.starts_with(s, prefix)` passes two boxed-string-object
//! ptrs through to `extern "C" fn rust_starts_with(*const u8,
//! *const u8) -> bool` in `src/bridge_runtime.rs`, then
//! truncates the i8 result back to i1 (Lua Bool).

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
fn starts_with_matching_prefix_returns_true() {
    assert_eq!(
        run_ok(
            "print(rust.starts_with(\"hello world\", \"hello\"))",
            "lumelir_bridge_sw_match"
        )
        .trim(),
        "true"
    );
}

#[test]
fn starts_with_non_matching_prefix_returns_false() {
    assert_eq!(
        run_ok(
            "print(rust.starts_with(\"hello world\", \"world\"))",
            "lumelir_bridge_sw_no_match"
        )
        .trim(),
        "false"
    );
}

#[test]
fn starts_with_empty_prefix_returns_true() {
    assert_eq!(
        run_ok(
            "print(rust.starts_with(\"any\", \"\"))",
            "lumelir_bridge_sw_empty_prefix"
        )
        .trim(),
        "true"
    );
}

#[test]
fn starts_with_prefix_longer_than_s_returns_false() {
    assert_eq!(
        run_ok(
            "print(rust.starts_with(\"hi\", \"hello\"))",
            "lumelir_bridge_sw_too_long"
        )
        .trim(),
        "false"
    );
}

#[test]
fn starts_with_composes_with_if_predicate() {
    let out = run_ok(
        "if rust.starts_with(\"prefix-rest\", \"prefix\") then
  print(\"matched\")
else
  print(\"missed\")
end",
        "lumelir_bridge_sw_in_if",
    );
    assert_eq!(out.trim(), "matched");
}
