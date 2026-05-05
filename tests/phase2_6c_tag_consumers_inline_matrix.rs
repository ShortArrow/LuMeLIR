//! Integration test: Phase 2.6c-tag-consumers-inline (ADR 0070)
//! — runtime tag dispatch for inline `type(t[k])` and
//! `tostring(t[k])` (no widening local in between). Mirrors the
//! ADR 0067 `Local(TaggedValue)` matrix for the inline form,
//! and resolves LIC-2.6c-tag-consumers-inline-1.

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
// `type(inline Index)` — runtime tag dispatch
// ============================================================

#[test]
fn type_inline_array_number() {
    let src = "local t = {1}
print(type(t[1]))";
    assert_eq!(run(src, "lumelir_inl_type_num").trim(), "number");
}

#[test]
fn type_inline_array_bool() {
    let src = "local t = {true}
print(type(t[1]))";
    assert_eq!(run(src, "lumelir_inl_type_bool").trim(), "boolean");
}

#[test]
fn type_inline_array_string() {
    let src = "local t = {\"a\"}
print(type(t[1]))";
    assert_eq!(run(src, "lumelir_inl_type_str").trim(), "string");
}

#[test]
fn type_inline_array_oob_nil() {
    let src = "local t = {1}
print(type(t[5]))";
    assert_eq!(run(src, "lumelir_inl_type_oob").trim(), "nil");
}

#[test]
fn type_inline_hash_string() {
    let src = "local t = {}
t.k = \"hi\"
print(type(t.k))";
    assert_eq!(run(src, "lumelir_inl_type_hash_str").trim(), "string");
}

#[test]
fn type_inline_hash_missing_nil() {
    let src = "local t = {}
print(type(t.absent))";
    assert_eq!(run(src, "lumelir_inl_type_hash_miss").trim(), "nil");
}

// ============================================================
// `tostring(inline Index)` — runtime tag dispatch
// ============================================================

#[test]
fn tostring_inline_array_number() {
    let src = "local t = {42}
print(tostring(t[1]))";
    assert_eq!(run(src, "lumelir_inl_ts_num").trim(), "42");
}

#[test]
fn tostring_inline_array_bool() {
    let src = "local t = {true}
print(tostring(t[1]))";
    assert_eq!(run(src, "lumelir_inl_ts_bool").trim(), "true");
}

#[test]
fn tostring_inline_array_string() {
    let src = "local t = {\"hi\"}
print(tostring(t[1]))";
    assert_eq!(run(src, "lumelir_inl_ts_str").trim(), "hi");
}

#[test]
fn tostring_inline_array_oob_nil() {
    let src = "local t = {1}
print(tostring(t[5]))";
    assert_eq!(run(src, "lumelir_inl_ts_oob").trim(), "nil");
}

// ============================================================
// `..` concat with inline Index — `tostring` auto-coerce
// per ADR 0026 routes through the new inline tag dispatch.
// ============================================================

#[test]
fn concat_inline_string() {
    let src = "local t = {\"hi\"}
print(\"v:\" .. t[1])";
    assert_eq!(run(src, "lumelir_inl_concat_str").trim(), "v:hi");
}

#[test]
fn concat_inline_bool() {
    let src = "local t = {true}
print(\"b:\" .. t[1])";
    assert_eq!(run(src, "lumelir_inl_concat_bool").trim(), "b:true");
}

// ============================================================
// Regression — ADR 0067 Local(TaggedValue) path stays green.
// ============================================================

#[test]
fn regression_type_local_tagged_string() {
    let src = "local t = {\"a\"}
local x = t[1]
print(type(x))";
    assert_eq!(run(src, "lumelir_inl_reg_type_local").trim(), "string");
}

#[test]
fn regression_tostring_local_tagged_bool() {
    let src = "local t = {true}
local x = t[1]
print(tostring(x))";
    assert_eq!(run(src, "lumelir_inl_reg_ts_local").trim(), "true");
}
