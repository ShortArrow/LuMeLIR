//! Integration test: Phase 2.6c-tag-hetero-fix — fix the two
//! P1 issues that codex review flagged on ADR 0064:
//!
//! 1. `print(t[k])` / `print(t.k)` traps on Bool / String / Nil
//!    payloads because the inline Index codegen still extracts
//!    f64 unconditionally. Fix: print arg dispatches Index
//!    through a tmp tagged slot.
//! 2. `TaggedValue == "literal"` constant-folds to `false` via
//!    the heterogeneous-kind fold (ADR 0061), even though the
//!    runtime tag may match. Fix: HIR fold skips when either
//!    side is `TaggedValue`; codegen emits a runtime tag-dispatch
//!    compare.

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

// =====================================================================
// Issue 1: inline `print(Index)` dispatches at runtime on the slot tag.
// =====================================================================

#[test]
fn inline_print_array_string() {
    let src = "local t = {\"hello\"}
print(t[1])";
    assert_eq!(run(src, "lumelir_fix_inline_arr_str").trim(), "hello");
}

#[test]
fn inline_print_array_bool() {
    let src = "local t = {true}
print(t[1])";
    assert_eq!(run(src, "lumelir_fix_inline_arr_bool").trim(), "true");
}

#[test]
fn inline_print_hash_string() {
    let src = "local t = {}
t.k = \"world\"
print(t.k)";
    assert_eq!(run(src, "lumelir_fix_inline_hash_str").trim(), "world");
}

#[test]
fn inline_print_hash_missing_returns_nil() {
    let src = "local t = {}
print(t.absent)";
    assert_eq!(run(src, "lumelir_fix_inline_hash_missing").trim(), "nil");
}

#[test]
fn inline_print_array_oob_returns_nil() {
    // ADR 0061 explicitly kept the inline OOB read trapping; ADR
    // 0065 supersedes that decision because under hetero values
    // the trap is wrong (Lua: returns nil → printed as "nil").
    let src = "local t = {1}
print(t[5])";
    assert_eq!(run(src, "lumelir_fix_inline_arr_oob").trim(), "nil");
}

// =====================================================================
// Issue 2: `TaggedValue == <literal>` runtime tag-dispatch.
// =====================================================================

#[test]
fn tagged_local_eq_string_literal_match() {
    let src = "local t = {\"a\"}
local x = t[1]
if x == \"a\" then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_fix_eq_str_match").trim(), "yes");
}

#[test]
fn tagged_local_eq_string_literal_mismatch() {
    let src = "local t = {\"a\"}
local x = t[1]
if x == \"b\" then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_fix_eq_str_miss").trim(), "no");
}

#[test]
fn tagged_local_ne_string_literal_match_inverted() {
    // Direct mirror of `eq_string_literal_match` via `~=`. The
    // pre-fix fold collapses `x ~= "a"` to true unconditionally
    // (heterogeneous → false on Eq → true via Ne). The post-fix
    // codegen agrees with the runtime tag.
    let src = "local t = {\"a\"}
local x = t[1]
if x ~= \"a\" then print(\"differ\") else print(\"same\") end";
    assert_eq!(run(src, "lumelir_fix_ne_str_match").trim(), "same");
}

#[test]
fn tagged_local_eq_bool_literal_match() {
    let src = "local t = {true}
local x = t[1]
if x == true then print(\"ok\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_fix_eq_bool_match").trim(), "ok");
}

#[test]
fn tagged_local_eq_number_literal_match() {
    let src = "local t = {42}
local x = t[1]
if x == 42 then print(\"forty-two\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_fix_eq_num_match").trim(), "forty-two");
}

#[test]
fn tagged_local_eq_wrong_kind_returns_false_at_runtime() {
    // x runtime tag is String, RHS is Number literal — runtime
    // tag mismatch → false.
    let src = "local t = {\"a\"}
local x = t[1]
if x == 1 then print(\"wrong\") else print(\"right\") end";
    assert_eq!(run(src, "lumelir_fix_eq_wrong_kind").trim(), "right");
}
