//! Integration test: Phase 2.6c-tag-fn-tbl (ADR 0071) —
//! Function and Table values stored as TaggedValue payloads in
//! tables (TAG_FUNCTION = 4, TAG_TABLE = 5). Closes the last
//! pure-pending entry, LIC-2.6c-tag-hetero-fn-tbl-1.
//!
//! Out of scope (separate LIC entries; see ADR 0071):
//! - Function with upvalues (closure escape) → HIR rejects.
//! - Calling a Function value retrieved through a tagged slot
//!   (`local f = t[1]; f()`) — tagged read is Number-only at
//!   the extract site (LIC-2.6c-tag-hetero-fn-tbl-call-1).

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
// Function values
// ============================================================

#[test]
fn function_in_array_construct_and_type() {
    let src = "local function f() return 1 end
local t = {f}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_fn_arr_type").trim(), "function");
}

#[test]
fn function_in_hash_construct_and_type() {
    let src = "local function f() return 1 end
local t = {}
t.f = f
print(type(t.f))";
    assert_eq!(run(src, "lumelir_fn_hash_type").trim(), "function");
}

#[test]
fn function_tostring_via_local() {
    let src = "local function f() return 1 end
local t = {f}
local x = t[1]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_fn_ts_local").trim(), "function");
}

#[test]
fn function_tostring_inline() {
    let src = "local function f() return 1 end
local t = {f}
print(tostring(t[1]))";
    assert_eq!(run(src, "lumelir_fn_ts_inline").trim(), "function");
}

#[test]
fn function_print_via_local() {
    let src = "local function f() return 1 end
local t = {f}
local x = t[1]
print(x)";
    assert_eq!(run(src, "lumelir_fn_print_local").trim(), "function");
}

// ============================================================
// Table values
// ============================================================

#[test]
fn table_in_array_construct_and_type() {
    let src = "local t = {{1, 2}, {3, 4}}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_tbl_arr_type").trim(), "table");
}

#[test]
fn table_in_hash_construct_and_type() {
    let src = "local t = {}
t.sub = {1, 2, 3}
print(type(t.sub))";
    assert_eq!(run(src, "lumelir_tbl_hash_type").trim(), "table");
}

#[test]
fn table_tostring_inline() {
    let src = "local t = {}
t.sub = {1, 2}
print(tostring(t.sub))";
    assert_eq!(run(src, "lumelir_tbl_ts_inline").trim(), "table");
}

#[test]
fn table_print_via_local() {
    let src = "local t = {}
t.sub = {1, 2}
local x = t.sub
print(x)";
    assert_eq!(run(src, "lumelir_tbl_print_local").trim(), "table");
}

// ============================================================
// Eq Local-Local (reference equality, Lua spec)
// ============================================================

#[test]
fn function_eq_same_ref() {
    let src = "local function f() return 1 end
local t = {f, f}
local a = t[1]
local b = t[2]
if a == b then print(\"same\") else print(\"diff\") end";
    assert_eq!(run(src, "lumelir_fn_eq_same").trim(), "same");
}

#[test]
fn function_eq_diff_ref() {
    let src = "local function f() return 1 end
local function g() return 2 end
local t = {f, g}
local a = t[1]
local b = t[2]
if a == b then print(\"same\") else print(\"diff\") end";
    assert_eq!(run(src, "lumelir_fn_eq_diff").trim(), "diff");
}

#[test]
fn table_eq_same_ref() {
    let src = "local u = {}
local t = {u, u}
local a = t[1]
local b = t[2]
if a == b then print(\"same\") else print(\"diff\") end";
    assert_eq!(run(src, "lumelir_tbl_eq_same").trim(), "same");
}

#[test]
fn table_eq_diff_ref() {
    let src = "local u = {}
local v = {}
local t = {u, v}
local a = t[1]
local b = t[2]
if a == b then print(\"same\") else print(\"diff\") end";
    assert_eq!(run(src, "lumelir_tbl_eq_diff").trim(), "diff");
}

// ============================================================
// Closure-with-upvalues escape — LIC-2.6c-tag-hetero-closure-
// escape-1 retired by ADR 0083 Commit 3c. The cell-ptr-first
// ABI (heap cell + heap upvalue boxes) makes table escapes
// sound, so the historical reject becomes a positive lower.
// ============================================================

#[test]
fn closure_with_upvalue_in_table_now_lowers_post_3c() {
    let chunk =
        lumelir::parser::parse("local x = 1\nlocal f = function() return x end\nlocal t = {f}")
            .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}

// ============================================================
// Regression — existing matrix paths stay green
// ============================================================

#[test]
fn regression_print_inline_string_still_works() {
    let src = "local t = {\"hi\"}
print(t[1])";
    assert_eq!(run(src, "lumelir_fn_reg_inline_str").trim(), "hi");
}
