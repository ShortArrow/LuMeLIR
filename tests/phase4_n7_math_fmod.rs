//! ADR 0262 — N7-1: `math.fmod(x, y)` via libm `fmod` (C-style
//! truncation remainder). Distinct from Lua's `%` operator (floor-mod).

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
fn math_fmod_positive() {
    assert_eq!(run_ok("print(math.fmod(7, 3))", "n7_fmod_pos").trim(), "1");
}

#[test]
fn math_fmod_negative_truncation() {
    // fmod truncates toward zero; -7 fmod 3 = -1 (NOT +2 like `%`).
    assert_eq!(
        run_ok("print(math.fmod(-7, 3))", "n7_fmod_neg").trim(),
        "-1"
    );
}

#[test]
fn math_fmod_zero_dividend() {
    assert_eq!(run_ok("print(math.fmod(0, 5))", "n7_fmod_zero").trim(), "0");
}
