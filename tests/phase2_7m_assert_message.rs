//! Integration test: Phase 2.7m — `assert(cond, msg)` with optional
//! custom error message (ADR 0051). Builds on Phase 2.7g.

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
fn assert_true_with_message_passes_silently() {
    let out = compile_and_run("assert(true, \"never\")\nprint(\"ok\")", "lumelir_27m_pass");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}

#[test]
fn assert_false_with_custom_message_exits_with_that_message() {
    let out = compile_and_run("assert(false, \"custom failure\")", "lumelir_27m_fail");
    assert!(!out.status.success(), "expected failure exit");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("custom failure"),
        "expected 'custom failure' in output, got: {combined}"
    );
}

#[test]
fn assert_false_without_message_uses_default_after_2_7m() {
    // Regression: 1-arg form keeps the default `assertion failed!`
    // wording from Phase 2.7g.
    let out = compile_and_run("assert(false)", "lumelir_27m_default");
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("assertion failed"),
        "expected 'assertion failed' in output, got: {combined}"
    );
}

#[test]
fn assert_message_can_be_a_local_string() {
    let src = "local why = \"locals work\"
assert(false, why)";
    let out = compile_and_run(src, "lumelir_27m_local_msg");
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("locals work"), "got: {combined}");
}

#[test]
fn assert_message_can_be_a_concat() {
    let src = "local n = 3
assert(false, \"value=\" .. n)";
    let out = compile_and_run(src, "lumelir_27m_concat");
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("value=3"),
        "expected 'value=3' in output, got: {combined}"
    );
}

#[test]
fn assert_zero_args_is_static_error() {
    let chunk = lumelir::parser::parse("assert()").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assert_three_args_is_static_error() {
    let chunk = lumelir::parser::parse("assert(true, \"a\", \"b\")").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assert_message_must_be_string() {
    // Number message is not allowed — Lua coerces tostring, but we
    // require explicit conversion.
    let chunk = lumelir::parser::parse("assert(false, 42)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
