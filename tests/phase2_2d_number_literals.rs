//! Integration test: Phase 2.2d — hex / decimal-float / scientific
//! number literals (ADR 0023).

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
fn hex_integer_lower_case_prefix() {
    assert_eq!(run("print(0xff)", "lumelir_22d_hex_lower").trim(), "255");
}

#[test]
fn hex_integer_upper_case_prefix() {
    assert_eq!(run("print(0X1A)", "lumelir_22d_hex_upper").trim(), "26");
}

#[test]
fn hex_integer_in_bitwise_expression() {
    assert_eq!(
        run("print(0xff & 0x0f)", "lumelir_22d_hex_band").trim(),
        "15"
    );
}

#[test]
fn decimal_float_prints_back_with_fraction() {
    assert_eq!(run("print(1.25)", "lumelir_22d_dec").trim(), "1.25");
}

#[test]
fn scientific_notation_no_fraction() {
    assert_eq!(run("print(1e3)", "lumelir_22d_e1").trim(), "1000");
}

#[test]
fn scientific_notation_signed_negative_exponent() {
    assert_eq!(run("print(2.5e-1)", "lumelir_22d_e2").trim(), "0.25");
}

#[test]
fn scientific_notation_signed_positive_exponent() {
    assert_eq!(run("print(2e+2)", "lumelir_22d_e3").trim(), "200");
}

#[test]
fn hex_in_shift_expression() {
    assert_eq!(run("print(0x10 << 4)", "lumelir_22d_hex_shl").trim(), "256");
}

#[test]
fn float_in_arithmetic() {
    let src = "local x = 1.5\nprint(x * 2)";
    assert_eq!(run(src, "lumelir_22d_float_mul").trim(), "3");
}

#[test]
fn integer_literal_unchanged_after_2_2d() {
    // Regression: plain decimal integers still work as before.
    assert_eq!(run("print(42)", "lumelir_22d_regress").trim(), "42");
}
