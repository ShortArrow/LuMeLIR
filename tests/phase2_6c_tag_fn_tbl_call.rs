//! Integration test: Phase 2.6c-tag-fn-tbl-call (ADR 0072,
//! superseded in part by ADR 0075).
//!
//! ADR 0072 originally enabled `local g = t[k]; g()` by
//! reconstructing the function type from `args.len()` at codegen
//! time. ADR 0075 (tagged-callee arity hardening, Strict Plan C)
//! determined this was an unsound `args.len()` reconstruction
//! that produced UB on arity / return-ABI mismatch, and rolled
//! the feature back: HIR now rejects every indirect call through
//! a TaggedValue local. The 6 originally-positive tests in this
//! file are reframed as negative reject tests; the 1 regression
//! test (direct-call via Function-kind local) is preserved.
//!
//! See `tests/phase2_6c_tag_callee_arity.rs` for the dedicated
//! ADR 0075 hardening test surface, including the heterogeneous-
//! arity hazard backstop.

use std::process::Command;

#[test]
fn array_indexed_no_arg_function_call_rejected_post_0075() {
    let chunk = lumelir::parser::parse(
        "local function f() return 1 end
local t = {f}
local g = t[1]
print(type(x))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn hash_indexed_no_arg_function_call_rejected_post_0075() {
    let chunk = lumelir::parser::parse(
        "local function f() return 1 end
local t = {}
t.f = f
local g = t.f
print(g())",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn array_indexed_function_call_with_args_rejected_post_0075() {
    let chunk = lumelir::parser::parse(
        "local function add(a, b) return a + b end
local t = {add}
local g = t[1]
print(g(3, 4))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn array_indexed_function_call_twice_rejected_post_0075() {
    let chunk = lumelir::parser::parse(
        "local function f() return 11 end
local t = {f, f}
local a = t[1]
local b = t[2]
print(a())
print(b())",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn heterogeneous_table_function_pick_call_rejected_post_0075() {
    // The original LIC-2.6c-tag-callee-arity-1 hazard: heterogeneous
    // table contents make any reconstruction unsound. Keep this as
    // an explicit backstop.
    let chunk = lumelir::parser::parse(
        "local function f() return 99 end
local t = {f, \"hi\", 1}
local g = t[1]
print(g())",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

// ============================================================
// Regression — direct-call via Function-kind local (the safe
// path) stays green.
// ============================================================

#[test]
fn regression_direct_function_local_call_still_works() {
    let output = std::env::temp_dir().join("lumelir_call_reg_direct");
    let chunk = lumelir::parser::parse(
        "local function f() return 5 end
local g = f
print(g())",
    )
    .unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "5");
}
