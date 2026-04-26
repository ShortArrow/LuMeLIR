//! Integration test: compile `print(1 + 2)` → native binary → run → stdout "3".

use std::process::Command;

#[test]
fn compile_and_run_print_1_plus_2() {
    let tmp_dir = std::env::temp_dir();
    let output = tmp_dir.join("lumelir_phase1_test");

    let expr = lumelir::parser::parse("print(1 + 2)").unwrap();
    lumelir::codegen::compile(&expr, &output).unwrap();

    assert!(output.exists(), "compiled binary should exist");

    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.trim(), "3", "expected '3', got '{}'", stdout.trim());
    assert!(result.status.success(), "binary should exit with 0");

    let _ = std::fs::remove_file(&output);
}

#[test]
fn compile_and_run_via_cli() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello.lua");
    let output = std::env::temp_dir().join("lumelir_phase1_cli_test");

    let status = Command::new(env!("CARGO_BIN_EXE_lumelir"))
        .args([
            "compile",
            fixture.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run lumelir compile");
    assert!(status.success(), "lumelir compile should succeed");

    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.trim(), "3");

    let _ = std::fs::remove_file(&output);
}
