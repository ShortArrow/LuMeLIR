//! Integration test: Phase 2.7h — `error(msg)` builtin (ADR 0033).

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

#[test]
fn error_string_literal_prints_message_then_exits_1() {
    let out = compile_and_run("error(\"oops\")", "lumelir_27h_lit");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("oops"),
        "expected message in output, got: {stdout}"
    );
}

#[test]
fn error_skips_following_statements() {
    let out = compile_and_run(
        "print(\"before\")
error(\"stop\")
print(\"never\")",
        "lumelir_27h_after",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("before"), "got: {stdout}");
    assert!(stdout.contains("stop"), "got: {stdout}");
    assert!(!stdout.contains("never"), "got: {stdout}");
}

#[test]
fn error_message_via_local_variable() {
    let out = compile_and_run(
        "local msg = \"bad input\"
error(msg)",
        "lumelir_27h_local",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bad input"), "got: {stdout}");
}

#[test]
fn error_message_via_concat_expression() {
    let out = compile_and_run(
        "local n = 42
error(\"got \" .. n)",
        "lumelir_27h_concat",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("got 42"), "got: {stdout}");
}

#[test]
fn error_inside_user_function_kills_caller_too() {
    let out = compile_and_run(
        "local function bail(s) error(s) end
bail(\"from-fn\")
print(\"unreachable\")",
        "lumelir_27h_in_fn",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("from-fn"), "got: {stdout}");
    assert!(!stdout.contains("unreachable"), "got: {stdout}");
}

#[test]
fn error_after_passing_assert() {
    // assert passes, error fires unconditionally afterwards.
    let out = compile_and_run(
        "assert(true)
error(\"boom\")",
        "lumelir_27h_with_assert",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("boom"), "got: {stdout}");
}
