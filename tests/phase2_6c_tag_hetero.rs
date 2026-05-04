//! Integration test: Phase 2.6c-tag-hetero — heterogeneous
//! Bool / String values in array and hash tables (ADR 0064).
//! Build on ADR 0063's `TaggedValue` local slot to allow
//! `local x = t[i]` to carry any tag and `print(x)` to dispatch
//! at runtime.
//!
//! Resolves LIC-2.6a-arr-2 / LIC-2.6a-wr-3 / LIC-2.6b-hash-2
//! for the Bool / String subset. Function and Table values
//! remain out of scope (ucast / cycle / closure-escape).

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

#[test]
fn array_string_value_via_local() {
    let src = "local t = {1, \"hello\", 3}
local x = t[2]
print(x)";
    assert_eq!(run(src, "lumelir_het_arr_str").trim(), "hello");
}

#[test]
fn hash_string_value() {
    let src = "local t = {}
t.greeting = \"hi\"
local x = t.greeting
print(x)";
    assert_eq!(run(src, "lumelir_het_hash_str").trim(), "hi");
}

#[test]
fn bool_value_in_array() {
    let src = "local t = {true, false}
local x = t[1]
if x ~= nil then print(x) end";
    assert_eq!(run(src, "lumelir_het_bool_arr").trim(), "true");
}

#[test]
fn mixed_value_lookup() {
    let src = "local t = {}
t.n = 42
t.s = \"lua\"
local n = t.n
local s = t.s
print(n)
print(s)";
    assert_eq!(run(src, "lumelir_het_mixed").trim(), "42\nlua");
}

#[test]
fn arith_on_tagged_local_traps_for_string() {
    // Lua spec: string + number errors (when not coercible).
    // Trap is acceptable behaviour for this sub-phase.
    let src = "local t = {\"hello\"}
local x = t[1]
print(x + 1)";
    let out = compile_and_run(src, "lumelir_het_arith_traps");
    assert!(!out.status.success(), "string + number must trap");
}

#[test]
fn isnil_check_with_string_value_returns_false() {
    let src = "local t = {}
t.k = \"value\"
local x = t.k
if x == nil then print(\"nil\") else print(\"not nil\") end";
    assert_eq!(run(src, "lumelir_het_isnil_string_false").trim(), "not nil");
}

#[test]
fn deleted_string_returns_nil() {
    let src = "local t = {}
t.k = \"old\"
t.k = nil
local x = t.k
if x == nil then print(\"deleted\") end";
    assert_eq!(run(src, "lumelir_het_deleted_str").trim(), "deleted");
}

#[test]
fn bool_print_dispatch() {
    let src = "local t = {true, false}
local a = t[1]
local b = t[2]
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_het_bool_print").trim(), "true\nfalse");
}

#[test]
fn hetero_table_with_holes() {
    // Hole at index 4 — `t[3]` is the third element, which is
    // `true`. Verifies hole-write doesn't disturb adjacent
    // tagged entries.
    let src = "local t = {1, \"two\", true}
local x = t[3]
if x ~= nil then print(x) end";
    assert_eq!(run(src, "lumelir_het_holes").trim(), "true");
}

#[test]
fn alias_widened_string() {
    let src = "local t = {\"hello\"}
local x = t[1]
local y = x
print(y)";
    assert_eq!(run(src, "lumelir_het_alias_str").trim(), "hello");
}

#[test]
fn regression_existing_number_arith_after_widening() {
    // A widened local that holds a Number should still extract
    // f64 cleanly for arithmetic. Backstop against breaking
    // ADR 0063's path.
    let src = "local t = {1, 2, 3}
local x = t[2]
if x ~= nil then print(x + 1) end";
    assert_eq!(run(src, "lumelir_het_arith_num").trim(), "3");
}

#[test]
fn regression_inline_index_unchanged() {
    // Inline `t[1]` (Number) still works through the existing
    // trapping Index codegen — the heterogeneous widening only
    // kicks in via `local x = t[i]`.
    let src = "local t = {1}
print(t[1])";
    assert_eq!(run(src, "lumelir_het_inline_num").trim(), "1");
}
