//! Phase 2.6+ param-string-context-inference (ADR 0181):
//! HIR infers a parameter as `ValueKind::String` when the body
//! uses it as a Concat operand or `string.*` method first arg.

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
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- Test 1: `param .. "x"` Concat operand (lhs side) ---

#[test]
fn param_used_with_concat_lhs_is_inferred_string() {
    let src = r#"
local function append_x(s) return s .. "x" end
local m = "hello"
print(append_x(m))
"#;
    let out = run_ok(src, "lumelir_param_concat_lhs");
    assert_eq!(out, "hellox\n");
}

// --- Test 2: `"x" .. param` Concat operand (rhs side) ---

#[test]
fn param_used_with_concat_rhs_is_inferred_string() {
    let src = r#"
local function prepend_x(s) return "x" .. s end
local m = "hello"
print(prepend_x(m))
"#;
    let out = run_ok(src, "lumelir_param_concat_rhs");
    assert_eq!(out, "xhello\n");
}

// --- Test 3: `string.upper(param)` Index-callee Call arg[0] ---

#[test]
fn param_used_with_string_method_is_inferred_string() {
    let src = r#"
local function up(s) return string.upper(s) end
local m = "hello"
print(up(m))
"#;
    let out = run_ok(src, "lumelir_param_string_method");
    assert_eq!(out, "HELLO\n");
}
