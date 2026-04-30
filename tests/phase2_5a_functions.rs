//! Integration test: Phase 2.5a — top-level local functions, return,
//! recursion. Number-only param/return.

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
fn void_function_call_runs_and_exits_zero() {
    // No output; binary just runs successfully.
    let src = "local function f() end\nf()";
    assert_eq!(run(src, "lumelir_25a_void"), "");
}

#[test]
fn no_arg_function_returns_constant() {
    let src = "local function one() return 1 end\nprint(one())";
    assert_eq!(run(src, "lumelir_25a_const").trim(), "1");
}

#[test]
fn two_arg_addition() {
    let src = "local function add(a, b) return a + b end\nprint(add(2, 3))";
    assert_eq!(run(src, "lumelir_25a_add").trim(), "5");
}

#[test]
fn factorial_via_recursion() {
    let src = "local function fact(n) if n == 0 then return 1 end\nreturn n * fact(n - 1) end\nprint(fact(5))";
    assert_eq!(run(src, "lumelir_25a_fact").trim(), "120");
}

#[test]
fn identity_function() {
    let src = "local function id(x) return x end\nprint(id(42))";
    assert_eq!(run(src, "lumelir_25a_id").trim(), "42");
}

#[test]
fn post_return_statements_skip() {
    // print(99) inside the function never runs because `return x`
    // sets the `_returned` guard before it.
    let src = "local function f(x) return x\nprint(99) end\nprint(f(7))";
    assert_eq!(run(src, "lumelir_25a_post_skip").trim(), "7");
}

#[test]
fn return_at_top_level_is_a_static_error() {
    let chunk = lumelir::parser::parse("return 1").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("top-level return must error");
    assert!(format!("{err}").contains("not inside a function"));
}

#[test]
fn break_inside_function_targets_inner_loop_only() {
    // The function's loop break exits the while, then the function
    // returns 99.
    let src = "local function f() while true do break end\nreturn 99 end\nprint(f())";
    assert_eq!(run(src, "lumelir_25a_break").trim(), "99");
}
