//! ADR 0272 — N7-11: os.execute(cmd) via libc system.

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
fn os_execute_true_returns_true() {
    let out = run_ok(
        "local ok = os.execute(\"true\")
if ok then print(\"yes\") else print(\"no\") end",
        "n7_exec_true",
    );
    assert_eq!(out.trim(), "yes");
}

#[test]
fn os_execute_false_returns_false() {
    let out = run_ok(
        "local ok = os.execute(\"false\")
if ok then print(\"yes\") else print(\"no\") end",
        "n7_exec_false",
    );
    assert_eq!(out.trim(), "no");
}
