//! Integration test: Phase 2.2c — floor division `//` and the five
//! bitwise operators (ADR 0022).

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn floor_div_positive_truncates_down() {
    assert_eq!(run("print(7 // 2)", "lumelir_22c_fdiv_pos").trim(), "3");
}

#[test]
fn floor_div_negative_floors_toward_minus_infinity() {
    // Lua: -7 // 2 == -4 (floor), not -3 (trunc).
    assert_eq!(run("print(-7 // 2)", "lumelir_22c_fdiv_neg").trim(), "-4");
}

#[test]
fn bitwise_and_masks_lower_nibble() {
    assert_eq!(run("print(15 & 5)", "lumelir_22c_band").trim(), "5");
}

#[test]
fn bitwise_or_sets_two_bits() {
    assert_eq!(run("print(1 | 2)", "lumelir_22c_bor").trim(), "3");
}

#[test]
fn bitwise_xor_via_tilde() {
    assert_eq!(run("print(5 ~ 3)", "lumelir_22c_bxor").trim(), "6");
}

#[test]
fn shift_left_doubles() {
    assert_eq!(run("print(1 << 3)", "lumelir_22c_shl").trim(), "8");
}

#[test]
fn shift_right_arithmetic_halves() {
    assert_eq!(run("print(16 >> 2)", "lumelir_22c_shr").trim(), "4");
}

#[test]
fn unary_bitnot_complements_zero() {
    assert_eq!(run("print(~0)", "lumelir_22c_bnot_zero").trim(), "-1");
}

#[test]
fn unary_bitnot_complements_one() {
    assert_eq!(run("print(~1)", "lumelir_22c_bnot_one").trim(), "-2");
}

#[test]
fn precedence_bitwise_below_shift() {
    // `1 | 2 << 3` parses as `1 | (2 << 3)` == `1 | 16` == 17.
    assert_eq!(run("print(1 | 2 << 3)", "lumelir_22c_prec1").trim(), "17");
}

#[test]
fn precedence_bitand_above_bitor() {
    // `1 | 2 & 0` parses as `1 | (2 & 0)` == `1 | 0` == 1.
    assert_eq!(run("print(1 | 2 & 0)", "lumelir_22c_prec2").trim(), "1");
}

#[test]
fn floor_div_in_local() {
    let src = "local q = 100 // 7\nprint(q)";
    assert_eq!(run(src, "lumelir_22c_fdiv_local").trim(), "14");
}

#[test]
fn bitwise_in_user_function() {
    let src = "local function pack(hi, lo) return hi << 4 | lo end
print(pack(3, 5))";
    assert_eq!(run(src, "lumelir_22c_pack").trim(), "53");
}
