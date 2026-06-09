//! Phase 2.6+ param-methodcall-string-dispatch (ADR 0183):
//! `s:<method>()` MethodCall sugar dispatches to the matching
//! `string.<method>` namespace builtin when the receiver is a
//! Local of `ValueKind::String`. The parameter inference fold-
//! through (ADR 0182 + 0181) refines the receiver to `String`
//! when the method name is a known `string.*` method.

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

// --- Test 1: `param:upper()` ---

#[test]
fn param_methodcall_string_upper_dispatches() {
    let src = r#"
local function up(s) return s:upper() end
print(up("hello"))
"#;
    let out = run_ok(src, "lumelir_param_method_upper");
    assert_eq!(out, "HELLO\n");
}

// --- Test 2: `param:lower()` ---

#[test]
fn param_methodcall_string_lower_dispatches() {
    let src = r#"
local function lo(s) return s:lower() end
print(lo("WORLD"))
"#;
    let out = run_ok(src, "lumelir_param_method_lower");
    assert_eq!(out, "world\n");
}

// --- Test 3: `param:len()` returns Number ---

#[test]
fn param_methodcall_string_len_returns_number() {
    let src = r#"
local function ln(s) return s:len() end
print(ln("hello"))
"#;
    let out = run_ok(src, "lumelir_param_method_len");
    assert_eq!(out, "5\n");
}
