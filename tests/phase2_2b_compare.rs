//! Integration test: Phase 2.2b — comparisons + boolean literals.

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
fn lt_true_case() {
    assert_eq!(run("print(1 < 2)", "lumelir_22b_lt_true").trim(), "true");
}

#[test]
fn lt_false_case() {
    assert_eq!(run("print(2 < 1)", "lumelir_22b_lt_false").trim(), "false");
}

#[test]
fn eq_eq_true_case() {
    assert_eq!(run("print(1 == 1)", "lumelir_22b_eq").trim(), "true");
}

#[test]
fn ne_false_case() {
    assert_eq!(run("print(1 ~= 1)", "lumelir_22b_ne").trim(), "false");
}

#[test]
fn gteq_boundary_case() {
    assert_eq!(run("print(2 >= 2)", "lumelir_22b_gteq").trim(), "true");
}

#[test]
fn print_true_literal() {
    assert_eq!(run("print(true)", "lumelir_22b_lit_t").trim(), "true");
}

#[test]
fn print_false_literal() {
    assert_eq!(run("print(false)", "lumelir_22b_lit_f").trim(), "false");
}

#[test]
fn nan_compares_unequal_to_itself() {
    // Ordered `oeq` semantics: NaN == NaN is false. Lua 5.4 matches.
    assert_eq!(run("print(0/0 == 0/0)", "lumelir_22b_nan").trim(), "false");
}
