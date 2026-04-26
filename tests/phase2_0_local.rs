//! Integration test: Phase 2.0 target — `local x = 1; print(x + 2)` prints "3".

use std::process::Command;

#[test]
fn compile_and_run_local_then_print() {
    let output = std::env::temp_dir().join("lumelir_phase2_0_local");

    let chunk = lumelir::parser::parse("local x = 1\nprint(x + 2)").unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();

    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.trim(), "3", "expected '3', got '{}'", stdout.trim());
    assert!(result.status.success());

    let _ = std::fs::remove_file(&output);
}

#[test]
fn compile_and_run_two_locals() {
    let output = std::env::temp_dir().join("lumelir_phase2_0_two_locals");

    let chunk = lumelir::parser::parse("local a = 10\nlocal b = 5\nprint(a + b)").unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();

    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(
        stdout.trim(),
        "15",
        "expected '15', got '{}'",
        stdout.trim()
    );

    let _ = std::fs::remove_file(&output);
}
