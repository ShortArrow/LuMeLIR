//! Integration test: Phase 2.5c.1 — top-level `local function` can
//! now capture chunk-level locals declared above it (lifts the
//! ADR-0037 limitation; documented in ADR 0042).

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
fn top_level_local_function_captures_chunk_local() {
    let src = "local x = 5
local function f() return x end
print(f())";
    assert_eq!(run(src, "lumelir_25c1_basic").trim(), "5");
}

#[test]
fn top_level_local_function_with_param_and_capture() {
    let src = "local m = 10
local function add(a) return a + m end
print(add(5))";
    assert_eq!(run(src, "lumelir_25c1_param").trim(), "15");
}

#[test]
fn top_level_local_function_captures_multiple() {
    let src = "local a = 3
local b = 4
local function sum() return a + b end
print(sum())";
    assert_eq!(run(src, "lumelir_25c1_multi").trim(), "7");
}

#[test]
fn top_level_local_function_arithmetic_chain_form() {
    // The "use anonymous form" workaround from ADR 0037 is gone;
    // the `local function` form works directly.
    let src = "local m = 10
local function calc(x) return x * m + 1 end
print(calc(3))";
    assert_eq!(run(src, "lumelir_25c1_chain").trim(), "31");
}

#[test]
fn top_level_local_function_with_no_capture_still_works() {
    // Regression: top-level functions that don't capture anything
    // continue to work exactly as before.
    let src = "local function double(n) return n * 2 end
print(double(21))";
    assert_eq!(run(src, "lumelir_25c1_nocap").trim(), "42");
}

#[test]
fn forward_ref_capture_is_static_error() {
    // The function's body is lowered at the FunctionDef position,
    // so `x` declared *after* the function isn't in scope. Lua
    // semantics: the local doesn't exist yet at the FunctionDef
    // statement.
    let chunk = lumelir::parser::parse(
        "local function f() return x end
local x = 5
print(f())",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn sibling_top_level_function_call_still_works() {
    // Forward-ref between sibling top-level functions still works
    // (signatures are registered in pass 1).
    let src = "local function a() return b() + 1 end
local function b() return 10 end
print(a())";
    assert_eq!(run(src, "lumelir_25c1_sibling").trim(), "11");
}

#[test]
fn capture_then_reassign_outer_propagates() {
    // Live-binding inside the chunk scope, same as ADR-0037
    // semantics for anonymous closures.
    let src = "local x = 1
local function f() return x end
x = 99
print(f())";
    assert_eq!(run(src, "lumelir_25c1_live").trim(), "99");
}
