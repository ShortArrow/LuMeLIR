//! ADR 0292 — N7-21: debug.traceback() stub.

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
fn traceback_returns_empty_string() {
    let out = run_ok("print(#debug.traceback())", "n7_21_len");
    assert_eq!(out.trim(), "0");
}

#[test]
fn traceback_type_is_string() {
    let out = run_ok("print(type(debug.traceback()))", "n7_21_type");
    assert_eq!(out.trim(), "string");
}

#[test]
fn traceback_call_does_not_crash() {
    let out = run_ok(
        "local t = debug.traceback()
print(\"ok\")",
        "n7_21_call",
    );
    assert_eq!(out.trim(), "ok");
}
