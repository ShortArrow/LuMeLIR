//! Integration test: Phase 2.5b.3 — returning functions as values
//! (ADR 0019).

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
fn get_doubler_returns_function_then_calls_it() {
    let src = "local function d(x) return x*2 end
local function gd() return d end
local f = gd()
print(f(5))";
    assert_eq!(run(src, "lumelir_25b3_get_doubler").trim(), "10");
}

#[test]
fn anon_function_returned_directly_then_called() {
    let src = "local function make() return function(x) return x+1 end end
local g = make()
print(g(7))";
    assert_eq!(run(src, "lumelir_25b3_make").trim(), "8");
}

#[test]
fn returned_function_passed_as_apply_arg() {
    // Compose Phase 2.5b.3 (return) with Phase 2.5b.2 (pass).
    let src = "local function d(x) return x*2 end
local function gd() return d end
local function apply(g, x) return g(x) end
local h = gd()
print(apply(h, 4))";
    assert_eq!(run(src, "lumelir_25b3_apply_returned").trim(), "8");
}

#[test]
fn arity_mismatch_on_returned_function_is_static_error() {
    // `g` is bound from a call returning Function(1); calling it
    // with 2 arguments must fail at HIR-time.
    let chunk = lumelir::parser::parse(
        "local function d(x) return x*2 end
local function gd() return d end
local g = gd()
print(g(1, 2))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn returning_two_arg_function() {
    let src = "local function add(a, b) return a+b end
local function get_add() return add end
local f = get_add()
print(f(3, 4))";
    assert_eq!(run(src, "lumelir_25b3_two_arg").trim(), "7");
}

#[test]
fn returning_zero_arg_function() {
    let src = "local function k() return 99 end
local function get_k() return k end
local f = get_k()
print(f())";
    assert_eq!(run(src, "lumelir_25b3_zero_arg").trim(), "99");
}

#[test]
fn number_return_still_works_after_2_5b3_changes() {
    // Regression: existing Number-returning functions must keep working.
    let src = "local function f(x) return x*x end
print(f(7))";
    assert_eq!(run(src, "lumelir_25b3_regress").trim(), "49");
}
