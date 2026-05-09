//! Phase 2.5c-full Commit 3c (ADR 0083 supersedes 0044) —
//! the legacy `HirError::ClosureEscapes` reject is gone now
//! that the cell-ptr-first ABI (3b body) makes capturing
//! closures heap-rooted (heap cell + heap upvalue boxes).
//!
//! What this file pins now:
//! - Positive: the patterns that ADR 0044 historically
//!   rejected (closure as arg / via alias / inline anonymous
//!   arg) all run end-to-end.
//! - Regressions: closure-less Function-kind args / returns
//!   still work (those never went through the closure path).
//!
//! The `phase2_5c3_capturing_e2e.rs` file owns the broader
//! Commit 3c TDD suite (box_sharing / make_adder / ...). This
//! file remains as a focused regression around the original
//! ADR 0044 surface.

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
fn passing_closure_with_upvalue_as_arg_now_runs() {
    // ADR 0044 historically rejected this; ADR 0083 Commit 3c
    // accepts it because the cell-ptr-first ABI threads the
    // heap cell through the call.
    let src = "local m = 10
local f = function(x) return x + m end
local function apply(g, x) return g(x) end
print(apply(f, 5))";
    assert_eq!(run(src, "lumelir_25c3_arg_pass").trim(), "15");
}

#[test]
fn aliasing_closure_then_passing_as_arg_now_runs() {
    let src = "local m = 10
local f = function(x) return x + m end
local g = f
local function apply(h, x) return h(x) end
print(apply(g, 5))";
    assert_eq!(run(src, "lumelir_25c3_alias_arg").trim(), "15");
}

#[test]
fn anonymous_closure_with_upvalue_inline_passed_now_runs() {
    let src = "local m = 10
local function apply(g, x) return g(x) end
print(apply(function(x) return x + m end, 5))";
    assert_eq!(run(src, "lumelir_25c3_inline_arg").trim(), "15");
}

#[test]
fn direct_call_of_closure_with_upvalue_still_works() {
    // Regression: 2.5c-min direct dispatch unaffected.
    let src = "local m = 10
local f = function(x) return x + m end
print(f(5))";
    assert_eq!(run(src, "lumelir_25c3_direct").trim(), "15");
}

#[test]
fn aliasing_closure_then_calling_directly_still_works() {
    // Regression: alias preserves func_id; direct dispatch keeps
    // working through the alias.
    let src = "local m = 10
local f = function(x) return x + m end
local g = f
print(g(5))";
    assert_eq!(run(src, "lumelir_25c3_alias_direct").trim(), "15");
}

#[test]
fn passing_function_without_upvalues_as_arg_still_works() {
    let src = "local f = function(x) return x * 2 end
local function apply(g, x) return g(x) end
print(apply(f, 5))";
    assert_eq!(run(src, "lumelir_25c3_no_upv_arg").trim(), "10");
}

#[test]
fn returning_function_without_upvalues_still_works() {
    let src = "local function d(x) return x * 2 end
local function get() return d end
local f = get()
print(f(5))";
    assert_eq!(run(src, "lumelir_25c3_no_upv_ret").trim(), "10");
}
