//! ADR 0307 — F2 close: direct i64 bitwise on int slots (exact
//! beyond 2^53) + coverage-boundary pins.

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
fn bitand_beyond_2p53_exact() {
    // maxinteger & 1 == 1 — exact only on the i64 path (the f64
    // round-trip saturates maxinteger before masking).
    let out = run_ok(
        "local m = math.maxinteger
local low = m & 1
print(low)",
        "f2c_and_big",
    );
    assert_eq!(out.trim(), "1");
}

#[test]
fn bitor_combines() {
    let out = run_ok(
        "local a = 12
local b = 3
local c = a | b
print(c)",
        "f2c_or",
    );
    assert_eq!(out.trim(), "15");
}

#[test]
fn bitxor_beyond_2p53() {
    // (2^53 + 1) ~ 1 flips the low bit exactly: 2^53.
    let out = run_ok(
        "local big = 9007199254740993
local x = big ~ 1
print(x)",
        "f2c_xor_big",
    );
    assert_eq!(out.trim(), "9007199254740992");
}

#[test]
fn mixed_arith_bitwise_tree() {
    let out = run_ok(
        "local a = 6
local b = (a + 2) & 12
print(b)",
        "f2c_mixed_tree",
    );
    assert_eq!(out.trim(), "8");
}
