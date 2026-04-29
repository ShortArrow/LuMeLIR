//! Integration test: Phase 2.3c — short-circuit `and`/`or`/`not`.

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
fn not_true_is_false() {
    assert_eq!(run("print(not true)", "lumelir_23c_not_t").trim(), "false");
}

#[test]
fn not_false_is_true() {
    assert_eq!(run("print(not false)", "lumelir_23c_not_f").trim(), "true");
}

#[test]
fn not_nil_is_true() {
    assert_eq!(run("print(not nil)", "lumelir_23c_not_nil").trim(), "true");
}

#[test]
fn not_zero_is_false_lua_specific() {
    // Lua: 0 is truthy, so `not 0` is false.
    assert_eq!(run("print(not 0)", "lumelir_23c_not_zero").trim(), "false");
}

#[test]
fn and_returns_rhs_when_lhs_truthy() {
    assert_eq!(
        run("print(true and false)", "lumelir_23c_and_tf").trim(),
        "false"
    );
}

#[test]
fn or_returns_lhs_when_lhs_truthy() {
    assert_eq!(
        run("print(true or false)", "lumelir_23c_or_tf").trim(),
        "true"
    );
}

#[test]
fn not_inside_if_inverts_branch() {
    // `not (1 < 2)` is false → else arm.
    assert_eq!(
        run(
            "if not (1 < 2) then print(1) else print(2) end",
            "lumelir_23c_not_if"
        )
        .trim(),
        "2"
    );
}

#[test]
fn and_chain_in_if_condition() {
    assert_eq!(
        run(
            "if (1 < 2) and (2 < 3) then print(true) end",
            "lumelir_23c_and_chain"
        )
        .trim(),
        "true"
    );
}
