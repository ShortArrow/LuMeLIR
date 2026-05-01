//! Integration test: Phase 2.7g — `assert(cond)` builtin (ADR 0030).

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    result
}

fn run_ok(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0, got {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn assert_true_passes_silently() {
    let stdout = run_ok(
        "assert(true)
print(\"after\")",
        "lumelir_27g_true",
    );
    assert_eq!(stdout.trim(), "after");
}

#[test]
fn assert_equals_predicate_passes() {
    let stdout = run_ok(
        "assert(1 == 1)
print(\"ok\")",
        "lumelir_27g_eq",
    );
    assert_eq!(stdout.trim(), "ok");
}

#[test]
fn assert_tonumber_predicate_passes() {
    let stdout = run_ok(
        "assert(tonumber(\"42\") == 42)
print(\"parsed\")",
        "lumelir_27g_parse",
    );
    assert_eq!(stdout.trim(), "parsed");
}

#[test]
fn assert_false_exits_with_status_1() {
    let out = compile_and_run(
        "print(\"before\")
assert(false)
print(\"never\")",
        "lumelir_27g_false",
    );
    assert!(!out.status.success(), "assert(false) must fail");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("before"),
        "code before assert must run, got: {stdout}"
    );
    assert!(
        stdout.contains("assertion failed!"),
        "diagnostic must appear, got: {stdout}"
    );
    assert!(
        !stdout.contains("never"),
        "code after assert must not run, got: {stdout}"
    );
}

#[test]
fn assert_inside_user_function_passes() {
    let stdout = run_ok(
        "local function check(n) assert(n > 0) return n * 2 end
print(check(5))",
        "lumelir_27g_fn",
    );
    assert_eq!(stdout.trim(), "10");
}

#[test]
fn assert_chain() {
    let stdout = run_ok(
        "assert(1 < 2)
assert(\"a\" == \"a\")
assert(true)
print(\"all-pass\")",
        "lumelir_27g_chain",
    );
    assert_eq!(stdout.trim(), "all-pass");
}

#[test]
fn assert_of_number_is_static_error() {
    let chunk = lumelir::parser::parse("assert(1)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assert_of_string_is_static_error() {
    let chunk = lumelir::parser::parse("assert(\"true\")").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assert_of_nil_is_static_error() {
    let chunk = lumelir::parser::parse("assert(nil)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
