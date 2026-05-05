//! Integration test: Phase 2.6c-tag-consumers — runtime tag
//! dispatch for `type(x)` and `tostring(x)` when `x` is a
//! `Local(TaggedValue)` (ADR 0067). Closes
//! LIC-2.6c-tag-locals-1 ('type(x)` returning static "number")
//! and provides a small matrix scaffold (consumer × runtime
//! tag) that future phases can extend.

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
// `type(Local(TaggedValue))` — runtime tag dispatch
// ============================================================

#[test]
fn type_tagged_local_number() {
    let src = "local t = {1}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_cm_type_num").trim(), "number");
}

#[test]
fn type_tagged_local_bool() {
    let src = "local t = {true}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_cm_type_bool").trim(), "boolean");
}

#[test]
fn type_tagged_local_string() {
    let src = "local t = {\"a\"}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_cm_type_str").trim(), "string");
}

#[test]
fn type_tagged_local_nil_via_oob() {
    let src = "local t = {1}
local x = t[5]
print(type(x))";
    assert_eq!(run(src, "lumelir_cm_type_nil").trim(), "nil");
}

// ============================================================
// `tostring(Local(TaggedValue))` — runtime tag dispatch
// ============================================================

#[test]
fn tostring_tagged_local_number() {
    let src = "local t = {42}
local x = t[1]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_cm_ts_num").trim(), "42");
}

#[test]
fn tostring_tagged_local_bool_true() {
    let src = "local t = {true}
local x = t[1]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_cm_ts_bool").trim(), "true");
}

#[test]
fn tostring_tagged_local_string() {
    let src = "local t = {\"hi\"}
local x = t[1]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_cm_ts_str").trim(), "hi");
}

#[test]
fn tostring_tagged_local_nil_via_oob() {
    let src = "local t = {1}
local x = t[5]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_cm_ts_nil").trim(), "nil");
}

// ============================================================
// `..` concat with Local(TaggedValue) — `tostring` auto-coerce
// per ADR 0026 routes through the new tag dispatch.
// ============================================================

#[test]
fn concat_tagged_local_number() {
    let src = "local t = {42}
local x = t[1]
print(\"v:\" .. x)";
    assert_eq!(run(src, "lumelir_cm_concat_num").trim(), "v:42");
}

#[test]
fn concat_tagged_local_string() {
    let src = "local t = {\"hi\"}
local x = t[1]
print(\"v:\" .. x)";
    assert_eq!(run(src, "lumelir_cm_concat_str").trim(), "v:hi");
}

// ============================================================
// Regression coverage — pre-existing tagged consumers stay green
// ============================================================

#[test]
fn regression_isnil_tagged_local() {
    let src = "local t = {}
local x = t.k
if x == nil then print(\"nil\") end";
    assert_eq!(run(src, "lumelir_cm_reg_isnil").trim(), "nil");
}

#[test]
fn regression_eq_local_literal() {
    let src = "local t = {\"a\"}
local x = t[1]
if x == \"a\" then print(\"ok\") end";
    assert_eq!(run(src, "lumelir_cm_reg_eq_lit").trim(), "ok");
}

#[test]
fn regression_print_tagged_local_bool() {
    let src = "local t = {true}
local x = t[1]
print(x)";
    assert_eq!(run(src, "lumelir_cm_reg_print_bool").trim(), "true");
}

#[test]
fn regression_inline_print_string() {
    // ADR 0065 inline path: `print(t[k])` materialises through a
    // tmp tagged slot. Must keep working alongside the new
    // type/tostring paths.
    let src = "local t = {\"hi\"}
print(t[1])";
    assert_eq!(run(src, "lumelir_cm_reg_inline_print").trim(), "hi");
}

#[test]
fn regression_number_arith_via_tagged_local() {
    let src = "local t = {1, 2}
local x = t[1]
local y = t[2]
print(x + y)";
    assert_eq!(run(src, "lumelir_cm_reg_arith").trim(), "3");
}
