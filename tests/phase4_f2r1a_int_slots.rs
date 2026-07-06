//! ADR 0304 — F2-R1-a: i64 slots for Integer locals under the
//! conservative chunk gate. Exact 64-bit round-trips for
//! literal-initialized integer locals read by print.

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
fn local_beyond_2p53_roundtrips_exactly() {
    let out = run_ok(
        "local big = 9007199254740993
print(big)",
        "f2r1a_big",
    );
    assert_eq!(out.trim(), "9007199254740993");
}

#[test]
fn maxinteger_literal_local_exact() {
    let out = run_ok(
        "local m = 9223372036854775807
print(m)",
        "f2r1a_max",
    );
    assert_eq!(out.trim(), "9223372036854775807");
}

#[test]
fn small_integers_unchanged() {
    let out = run_ok(
        "local x = 5
print(x)",
        "f2r1a_small",
    );
    assert_eq!(out.trim(), "5");
}

#[test]
fn reassignment_keeps_integer_slot() {
    let out = run_ok(
        "local x = 9007199254740993
x = 9007199254740995
print(x)",
        "f2r1a_reassign",
    );
    assert_eq!(out.trim(), "9007199254740995");
}

#[test]
fn gate_bails_on_arithmetic_chunk() {
    // Chunk contains a BinOp store → gate bails, f64 behavior
    // preserved (no regression; R1-b widens this).
    let out = run_ok(
        "local x = 5
local y = x + 1
print(y)",
        "f2r1a_bail",
    );
    assert_eq!(out.trim(), "6");
}
