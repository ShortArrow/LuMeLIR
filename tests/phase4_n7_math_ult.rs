//! ADR 0274 — N7-13: math.ult unsigned 64-bit less-than.

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
fn math_ult_basic_true() {
    let out = run_ok(
        "if math.ult(1, 2) then print(\"yes\") else print(\"no\") end",
        "n7_ult_basic_true",
    );
    assert_eq!(out.trim(), "yes");
}

#[test]
fn math_ult_basic_false() {
    let out = run_ok(
        "if math.ult(5, 2) then print(\"yes\") else print(\"no\") end",
        "n7_ult_basic_false",
    );
    assert_eq!(out.trim(), "no");
}

#[test]
fn math_ult_negative_is_large_unsigned() {
    // -1 as i64 = 0xFFFF... so unsigned-greater-than any positive.
    // math.ult(-1, 5) treats -1 as 2^64-1 ⇒ false.
    let out = run_ok(
        "if math.ult(-1, 5) then print(\"yes\") else print(\"no\") end",
        "n7_ult_neg",
    );
    assert_eq!(out.trim(), "no");
}
