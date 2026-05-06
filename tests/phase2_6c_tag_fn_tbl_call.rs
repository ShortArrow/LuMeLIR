//! Integration test: Phase 2.6c-tag-fn-tbl-call (ADR 0072) →
//! ADR 0075 (rejected) → ADR 0082 (re-enabled via static dispatch).
//!
//! ADR 0072 originally enabled `local g = t[k]; g()` by reconstructing
//! the function type from `args.len()` at codegen time — that was UB
//! on heterogeneous tables (LIC-2.6c-tag-callee-arity-1). ADR 0075
//! hardened by rejecting the path entirely. ADR 0082 reopens it
//! safely: HIR computes a compile-time compatible-set of user
//! functions whose signature matches the call site, and codegen
//! emits a per-call-site `if loaded_ptr == @user_fn_X then func.call
//! @user_fn_X(...)` chain. The originally-positive tests are
//! reframed back to positive coverage of the dispatch path; the
//! direct-call regression test is preserved.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(result.status.success(), "binary should exit 0: {result:?}");
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn array_indexed_no_arg_function_call_dispatches() {
    let src = "local function f() return 1 end
local t = {f}
local g = t[1]
print(g())";
    assert_eq!(run(src, "lumelir_call_arr_no_arg").trim(), "1");
}

#[test]
fn hash_indexed_no_arg_function_call_dispatches() {
    let src = "local function f() return 1 end
local t = {}
t.f = f
local g = t.f
print(g())";
    assert_eq!(run(src, "lumelir_call_hash_no_arg").trim(), "1");
}

#[test]
fn array_indexed_function_call_with_args_dispatches() {
    let src = "local function add(a, b) return a + b end
local t = {add}
local g = t[1]
print(g(3, 4))";
    assert_eq!(run(src, "lumelir_call_arr_args").trim(), "7");
}

#[test]
fn array_indexed_function_call_twice_dispatches() {
    let src = "local function f() return 11 end
local t = {f, f}
local a = t[1]
local b = t[2]
print(a())
print(b())";
    assert_eq!(run(src, "lumelir_call_arr_twice").trim(), "11\n11");
}

#[test]
fn heterogeneous_table_function_pick_dispatches() {
    // ADR 0082: even with heterogeneous *value* kinds in the table,
    // the call site's expected signature (single-arg `() → Number`
    // here) restricts the candidate set to user functions matching
    // that ABI. The runtime tag check rejects non-function payloads
    // before the dispatch chain runs.
    let src = "local function f() return 99 end
local t = {f, \"hi\", 1}
local g = t[1]
print(g())";
    assert_eq!(run(src, "lumelir_call_hetero_pick").trim(), "99");
}

// ============================================================
// Regression — direct-call via Function-kind local (the safe
// path) stays green.
// ============================================================

#[test]
fn regression_direct_function_local_call_still_works() {
    let src = "local function f() return 5 end
local g = f
print(g())";
    assert_eq!(run(src, "lumelir_call_reg_direct").trim(), "5");
}
