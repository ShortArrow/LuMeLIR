//! ADR 0264 — N7-3: `os.exit([code])` via libc `exit(int)`.

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::ExitStatus {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .status()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    r
}

#[test]
fn os_exit_zero_default() {
    let s = compile_and_run("os.exit()", "n7_osexit_default");
    assert_eq!(s.code(), Some(0));
}

#[test]
fn os_exit_specific_code() {
    let s = compile_and_run("os.exit(42)", "n7_osexit_42");
    assert_eq!(s.code(), Some(42));
}

#[test]
fn os_exit_aborts_following_statements() {
    // Anything after os.exit must not run.
    let s = compile_and_run(
        "os.exit(7)
print(\"should not print\")",
        "n7_osexit_diverges",
    );
    assert_eq!(s.code(), Some(7));
}
