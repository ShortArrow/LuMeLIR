//! Integration test: Phase 2.6c-tag-hetero-eq — `Local(TaggedValue)
//! == Local(TaggedValue)` runtime tag dispatch (ADR 0066).
//! Closes the last LIC entry from ADR 0064 / 0065
//! (LIC-2.6c-tag-hetero-eq-1).

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
fn local_local_string_eq_match() {
    let src = "local t = {\"a\", \"a\"}
local x = t[1]
local y = t[2]
if x == y then print(\"equal\") else print(\"differ\") end";
    assert_eq!(run(src, "lumelir_eq_ll_str_match").trim(), "equal");
}

#[test]
fn local_local_string_eq_mismatch() {
    let src = "local t = {\"a\", \"b\"}
local x = t[1]
local y = t[2]
if x == y then print(\"equal\") else print(\"differ\") end";
    assert_eq!(run(src, "lumelir_eq_ll_str_miss").trim(), "differ");
}

#[test]
fn local_local_number_eq_match() {
    let src = "local t = {42, 42}
local x = t[1]
local y = t[2]
if x == y then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_eq_ll_num").trim(), "yes");
}

#[test]
fn local_local_bool_eq_match() {
    let src = "local t = {true, true}
local x = t[1]
local y = t[2]
if x == y then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_eq_ll_bool").trim(), "yes");
}

#[test]
fn local_local_kind_mismatch_returns_false() {
    let src = "local t = {\"a\", 1}
local x = t[1]
local y = t[2]
if x == y then print(\"eq\") else print(\"ne\") end";
    assert_eq!(run(src, "lumelir_eq_ll_kind_miss").trim(), "ne");
}

#[test]
fn local_local_both_nil_eq_true() {
    // Both reads OOB → Nil tag. Lua spec: nil == nil is true.
    let src = "local t = {1, 2}
local x = t[5]
local y = t[6]
if x == y then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_eq_ll_both_nil").trim(), "yes");
}

#[test]
fn local_local_ne_inverted() {
    let src = "local t = {\"a\", \"b\"}
local x = t[1]
local y = t[2]
if x ~= y then print(\"differ\") else print(\"same\") end";
    assert_eq!(run(src, "lumelir_eq_ll_ne").trim(), "differ");
}

#[test]
fn alias_eq() {
    let src = "local t = {\"hi\"}
local x = t[1]
local y = x
if x == y then print(\"alias\") else print(\"split\") end";
    assert_eq!(run(src, "lumelir_eq_ll_alias").trim(), "alias");
}

#[test]
fn regression_local_literal_still_works() {
    // ADR 0065 path (Local-Literal) must keep working alongside
    // the new Local-Local path.
    let src = "local t = {\"a\"}
local x = t[1]
if x == \"a\" then print(\"ok\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_eq_ll_lit_regression").trim(), "ok");
}

#[test]
fn regression_isnil_still_works() {
    // After the IsNil unification (Tidy First), the basic
    // `local x = t[oob]; x == nil` form must remain green.
    let src = "local t = {1}
local x = t[5]
if x == nil then print(\"nil\") end";
    assert_eq!(run(src, "lumelir_eq_ll_isnil_regression").trim(), "nil");
}
