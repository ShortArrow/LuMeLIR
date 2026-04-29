//! Integration test: Phase 2.3a — `nil`, per-slot type tracking,
//! heterogeneous `==` static fold.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn print_nil_literal() {
    assert_eq!(run("print(nil)", "lumelir_23a_nil_lit").trim(), "nil");
}

#[test]
fn local_nil_then_print() {
    assert_eq!(
        run("local n = nil\nprint(n)", "lumelir_23a_local_nil").trim(),
        "nil"
    );
}

#[test]
fn local_bool_true_then_print() {
    // Phase 2.2b deferred this to 2.3a — now it must work.
    assert_eq!(
        run("local b = true\nprint(b)", "lumelir_23a_local_t").trim(),
        "true"
    );
}

#[test]
fn local_bool_false_then_print() {
    assert_eq!(
        run("local b = false\nprint(b)", "lumelir_23a_local_f").trim(),
        "false"
    );
}

#[test]
fn nil_eq_nil_is_true() {
    assert_eq!(run("print(nil == nil)", "lumelir_23a_nn").trim(), "true");
}

#[test]
fn number_eq_nil_is_false() {
    assert_eq!(run("print(1 == nil)", "lumelir_23a_1n").trim(), "false");
}

#[test]
fn number_eq_bool_is_false() {
    // Heterogeneous == was rejected as TypeMismatch in 2.2b; ADR 0011
    // promotes it to Lua's static-false fold.
    assert_eq!(run("print(1 == true)", "lumelir_23a_1t").trim(), "false");
}

#[test]
fn nil_ne_nil_is_false() {
    assert_eq!(run("print(nil ~= nil)", "lumelir_23a_nnne").trim(), "false");
}
