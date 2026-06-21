//! ADR 0240 — M11-A: math.* unary expansion sweep. Adds `ceil`,
//! `tan`, `asin`, `acos`, `atan` via libm extern declarations
//! sharing the existing f64 → f64 dispatch shape (ADR 0102).

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
fn math_ceil_positive_fractional() {
    assert_eq!(
        run_ok("print(math.ceil(3.2))", "lumelir_m11a_ceil_pos").trim(),
        "4"
    );
}

#[test]
fn math_ceil_negative_fractional() {
    assert_eq!(
        run_ok("print(math.ceil(-3.2))", "lumelir_m11a_ceil_neg").trim(),
        "-3"
    );
}

#[test]
fn math_ceil_integer_is_identity() {
    assert_eq!(
        run_ok("print(math.ceil(7))", "lumelir_m11a_ceil_int").trim(),
        "7"
    );
}

#[test]
fn math_tan_zero_is_zero() {
    assert_eq!(
        run_ok("print(math.tan(0))", "lumelir_m11a_tan_zero").trim(),
        "0"
    );
}

#[test]
fn math_asin_zero_is_zero() {
    assert_eq!(
        run_ok("print(math.asin(0))", "lumelir_m11a_asin_zero").trim(),
        "0"
    );
}

#[test]
fn math_acos_one_is_zero() {
    assert_eq!(
        run_ok("print(math.acos(1))", "lumelir_m11a_acos_one").trim(),
        "0"
    );
}

#[test]
fn math_atan_zero_is_zero() {
    assert_eq!(
        run_ok("print(math.atan(0))", "lumelir_m11a_atan_zero").trim(),
        "0"
    );
}

#[test]
fn math_atan_one_is_pi_over_four() {
    // atan(1) = pi/4 ≈ 0.785398.
    let out = run_ok("print(math.atan(1))", "lumelir_m11a_atan_one");
    let s = out.trim();
    // %g format yields ~6 significant digits.
    assert!(
        s.starts_with("0.7853"),
        "expected pi/4 ≈ 0.7853..., got: {s}"
    );
}
