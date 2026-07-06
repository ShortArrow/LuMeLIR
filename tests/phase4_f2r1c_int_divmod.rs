//! ADR 0306 — F2-R1-c (step 1): i64 floor-div / mod with Lua
//! floor semantics + zero-divisor trap.

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
fn floordiv_negative_floors() {
    // Lua: -7 // 2 == -4 (floors toward -inf, not toward zero).
    let out = run_ok(
        "local a = -7
local q = a // 2
print(q)",
        "f2r1c_fd_neg",
    );
    assert_eq!(out.trim(), "-4");
}

#[test]
fn floordiv_positive() {
    let out = run_ok(
        "local a = 7
local q = a // 2
print(q)",
        "f2r1c_fd_pos",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn mod_sign_follows_divisor() {
    // Lua: -7 % 2 == 1 (result sign follows divisor).
    let out = run_ok(
        "local a = -7
local m = a % 2
print(m)",
        "f2r1c_mod_neg",
    );
    assert_eq!(out.trim(), "1");
}

#[test]
fn mod_negative_divisor() {
    // Lua: 7 % -2 == -1.
    let out = run_ok(
        "local a = 7
local m = a % -2
print(m)",
        "f2r1c_mod_negdiv",
    );
    assert_eq!(out.trim(), "-1");
}

#[test]
fn int_div_zero_traps() {
    let src = "local a = 7
local z = 0
local q = a // z
print(q)";
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    let output = std::env::temp_dir().join("f2r1c_div0");
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output).output().expect("run failed");
    let _ = std::fs::remove_file(&output);
    assert!(!r.status.success(), "expected trap on n//0: {r:?}");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
    assert!(
        all.contains("integer division by zero"),
        "wrong message: {all}"
    );
}

#[test]
fn wraparound_equality_end_to_end() {
    // The full ADR 0300 probe: maxinteger + 1 == mininteger.
    let out = run_ok(
        "local m = math.maxinteger
local w = m + 1
local mn = math.mininteger
local eq = w == mn
print(eq)",
        "f2r1c_wrap_eq",
    );
    assert_eq!(out.trim(), "true");
}

#[test]
fn int_comparison_lt() {
    let out = run_ok(
        "local a = 9007199254740993
local b = 9007199254740992
local lt = b < a
print(lt)",
        "f2r1c_lt",
    );
    assert_eq!(out.trim(), "true");
}
