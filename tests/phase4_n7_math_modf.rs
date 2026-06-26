//! ADR 0275 — N7-14: math.modf integer part (single-result scope).

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
fn math_modf_positive() {
    let out = run_ok("print(math.modf(3.75))", "n7_modf_pos");
    assert_eq!(out.trim(), "3");
}

#[test]
fn math_modf_negative_truncates_toward_zero() {
    // libm trunc rounds toward zero: trunc(-2.6) = -2.
    let out = run_ok("print(math.modf(-2.6))", "n7_modf_neg");
    assert_eq!(out.trim(), "-2");
}

#[test]
fn math_modf_integer_input_unchanged() {
    let out = run_ok("print(math.modf(7))", "n7_modf_int");
    assert_eq!(out.trim(), "7");
}
