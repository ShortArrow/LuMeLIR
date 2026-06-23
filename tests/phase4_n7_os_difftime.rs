//! ADR 0269 — N7-8: os.difftime(t2, t1) = t2 - t1 inline.

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
fn os_difftime_positive() {
    assert_eq!(
        run_ok("print(os.difftime(100, 60))", "n7_difftime_pos").trim(),
        "40"
    );
}

#[test]
fn os_difftime_negative() {
    assert_eq!(
        run_ok("print(os.difftime(5, 10))", "n7_difftime_neg").trim(),
        "-5"
    );
}

#[test]
fn os_difftime_zero() {
    assert_eq!(
        run_ok("print(os.difftime(7, 7))", "n7_difftime_zero").trim(),
        "0"
    );
}
