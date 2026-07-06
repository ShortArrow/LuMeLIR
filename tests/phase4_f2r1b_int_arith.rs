//! ADR 0305 — F2-R1-b: i64 arithmetic on int slots. Fixes both
//! ADR 0300 R1 probes: wraparound + >2^53 runtime precision.

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
fn maxinteger_plus_one_wraps_to_mininteger() {
    let out = run_ok(
        "local m = math.maxinteger
local w = m + 1
print(w)",
        "f2r1b_wrap",
    );
    assert_eq!(out.trim(), "-9223372036854775808");
}

#[test]
fn runtime_add_beyond_2p53_exact() {
    let out = run_ok(
        "local big = 9007199254740992
local b1 = big + 1
print(b1)",
        "f2r1b_big_add",
    );
    assert_eq!(out.trim(), "9007199254740993");
}

#[test]
fn mul_and_sub_trees() {
    let out = run_ok(
        "local a = 3
local b = 4
local c = a * b - 2
print(c)",
        "f2r1b_tree",
    );
    assert_eq!(out.trim(), "10");
}

#[test]
fn mininteger_minus_one_wraps_to_maxinteger() {
    let out = run_ok(
        "local m = math.mininteger
local w = m - 1
print(w)",
        "f2r1b_wrap_down",
    );
    assert_eq!(out.trim(), "9223372036854775807");
}

#[test]
fn gate_bails_on_division() {
    // `/` is always Float — the chunk bails and old f64 behavior
    // holds (documented; FloorDiv/Mod i64 semantics are R1-c).
    let out = run_ok(
        "local x = 10
local y = x / 4
print(y)",
        "f2r1b_div_bail",
    );
    assert_eq!(out.trim(), "2.5");
}
