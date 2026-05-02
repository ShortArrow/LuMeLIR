//! Integration test: Phase 2.5c.2 — extend capture-by-value
//! closures to Bool, Nil, and String upvalues (ADR 0043).
//! Function-kind upvalues remain rejected.

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
fn bool_upvalue_can_be_captured_and_returned() {
    let src = "local b = true
local function f() return b end
print(f())";
    assert_eq!(run(src, "lumelir_25c2_bool").trim(), "true");
}

#[test]
fn bool_upvalue_drives_branch() {
    let src = "local flag = true
local function pick()
  if flag then return 1 else return 2 end
end
print(pick())";
    assert_eq!(run(src, "lumelir_25c2_bool_branch").trim(), "1");
}

#[test]
fn string_upvalue_can_be_captured_and_returned() {
    let src = "local s = \"hello\"
local function f() return s end
print(f())";
    assert_eq!(run(src, "lumelir_25c2_str").trim(), "hello");
}

#[test]
fn string_upvalue_can_be_concatenated() {
    let src = "local greeting = \"hi\"
local function emit(name) return greeting .. \" \" .. name end
print(emit(\"world\"))";
    assert_eq!(run(src, "lumelir_25c2_str_concat").trim(), "hi world");
}

#[test]
fn nil_upvalue_can_be_captured_and_returned() {
    let src = "local n = nil
local function f() return n end
print(f())";
    assert_eq!(run(src, "lumelir_25c2_nil").trim(), "nil");
}

#[test]
fn mixed_number_and_string_captures() {
    let src = "local label = \"score\"
local total = 42
local function show() return label .. \" = \" .. total end
print(show())";
    assert_eq!(run(src, "lumelir_25c2_mixed").trim(), "score = 42");
}

#[test]
fn function_upvalue_remains_rejected() {
    // Function-kind captures still error — codegen doesn't yet
    // bridge the (fn_ptr) upvalue path.
    let chunk = lumelir::parser::parse(
        "local g = function(x) return x + 1 end
local function f() return g end
print(f()(5))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn nested_bool_capture_in_inner_function() {
    // The kind expansion applies to the nested-function path too.
    let src = "local function outer()
  local b = true
  local function inner() return b end
  return inner()
end
print(outer())";
    assert_eq!(run(src, "lumelir_25c2_nested_bool").trim(), "true");
}
