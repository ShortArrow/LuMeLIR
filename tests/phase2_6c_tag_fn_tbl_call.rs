//! Integration test: Phase 2.6c-tag-fn-tbl-call (ADR 0072) —
//! call a Function value retrieved through a tagged slot
//! (`local f = t[1]; f()` and friends). Closes
//! LIC-2.6c-tag-hetero-fn-tbl-call-1.
//!
//! Out of scope (separate LIC entries):
//! - Function-typed return widening (`local x = f()` where `f`
//!   returns mixed kinds) — LIC tracked under function-return
//!   widening (Codex roadmap #3).
//! - Closure with upvalues retrieved through a tagged slot —
//!   still HIR-rejected at table-storage time
//!   (LIC-2.6c-tag-hetero-closure-escape-1).

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

fn run(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0, got {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ============================================================
// Call through tagged-slot Local
// ============================================================

#[test]
fn call_function_from_array_tagged_local_no_args() {
    let src = "local function f() return 42 end
local t = {f}
local g = t[1]
print(g())";
    assert_eq!(run(src, "lumelir_call_tagged_arr_noargs").trim(), "42");
}

#[test]
fn call_function_from_hash_tagged_local_no_args() {
    let src = "local function f() return 7 end
local t = {}
t.f = f
local g = t.f
print(g())";
    assert_eq!(run(src, "lumelir_call_tagged_hash_noargs").trim(), "7");
}

#[test]
fn call_function_from_tagged_local_with_args() {
    let src = "local function add(a, b) return a + b end
local t = {add}
local g = t[1]
print(g(3, 4))";
    assert_eq!(run(src, "lumelir_call_tagged_with_args").trim(), "7");
}

#[test]
fn call_function_from_tagged_local_twice() {
    let src = "local function f() return 11 end
local t = {f, f}
local a = t[1]
local b = t[2]
print(a())
print(b())";
    assert_eq!(run(src, "lumelir_call_tagged_twice").trim(), "11\n11");
}

#[test]
fn call_function_picked_among_heterogeneous_table() {
    let src = "local function f() return 99 end
local t = {f, \"hi\", 1}
local g = t[1]
print(g())";
    assert_eq!(run(src, "lumelir_call_tagged_hetero").trim(), "99");
}

// ============================================================
// Runtime tag-mismatch: calling a non-Function tagged value
// must fail-fast (trap), not silently misbehave.
// ============================================================

#[test]
fn call_non_function_tagged_local_traps() {
    let src = "local t = {1, 2}
local g = t[1]
print(g())";
    let out = compile_and_run(src, "lumelir_call_tagged_trap_number");
    assert!(
        !out.status.success(),
        "calling a Number tagged-slot value must trap, got exit 0"
    );
}

// ============================================================
// Regression: existing direct-Function-local call paths
// (Phase 2.5b.2) stay green.
// ============================================================

#[test]
fn regression_direct_function_local_call_still_works() {
    let src = "local function f() return 5 end
local g = f
print(g())";
    assert_eq!(run(src, "lumelir_call_reg_direct").trim(), "5");
}
