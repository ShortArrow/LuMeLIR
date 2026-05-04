//! Integration test: Phase 2.6c-tag-locals — `local x = t[i]`
//! widens x into a 16-byte tagged slot (`MaybeNil(Number)`)
//! so that `if x == nil`-style nil checks work after a local
//! binding (ADR 0063). The sister phase to ADR 0061: that one
//! handled inline `t[i] == nil`, this one handles
//! `local x = t[i]; if x == nil ...`.
//!
//! Resolves the locals form of LIC-2.6a-arr-1 and
//! LIC-2.6b-hash-1.

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
fn array_oob_via_local() {
    let src = "local t = {1, 2, 3}
local x = t[5]
if x == nil then print(\"oob\") end";
    assert_eq!(run(src, "lumelir_locw_oob").trim(), "oob");
}

#[test]
fn array_in_bounds_local_ne_nil() {
    let src = "local t = {10, 20, 30}
local x = t[2]
if x ~= nil then print(x) end";
    assert_eq!(run(src, "lumelir_locw_inbounds_ne").trim(), "20");
}

#[test]
fn hash_missing_via_local() {
    let src = "local t = {}
local x = t.foo
if x == nil then print(\"missing\") end";
    assert_eq!(run(src, "lumelir_locw_missing").trim(), "missing");
}

#[test]
fn hash_present_via_local() {
    let src = "local t = {}
t.k = 42
local x = t.k
if x ~= nil then print(x) end";
    assert_eq!(run(src, "lumelir_locw_present").trim(), "42");
}

#[test]
fn deleted_then_local_query() {
    let src = "local t = {}
t.k = 1
t.k = nil
local x = t.k
if x == nil then print(\"deleted\") end";
    assert_eq!(run(src, "lumelir_locw_deleted").trim(), "deleted");
}

#[test]
fn arith_after_nil_check() {
    let src = "local t = {1, 2, 3}
local x = t[2]
if x ~= nil then print(x + 1) end";
    assert_eq!(run(src, "lumelir_locw_arith_after").trim(), "3");
}

#[test]
fn plain_arith_with_nil_traps() {
    // Lua semantics: nil + 1 is a runtime error. The widened
    // local extracts via tag check, so an arithmetic use of a
    // nil-tagged local traps the same way `print(t[oob]) + 1`
    // would in inline form.
    let src = "local t = {1, 2}
local x = t[5]
print(x + 1)";
    let out = compile_and_run(src, "lumelir_locw_arith_traps");
    assert!(!out.status.success(), "arith on nil-tagged must trap");
}

#[test]
fn reassign_widened_local_to_oob() {
    let src = "local t = {1, 2, 3}
local x = t[1]
x = t[5]
if x == nil then print(\"nil after reassign\") end";
    assert_eq!(
        run(src, "lumelir_locw_reassign").trim(),
        "nil after reassign"
    );
}

#[test]
fn alias_widened_local_propagates_kind() {
    let src = "local t = {1, 2, 3}
local x = t[5]
local y = x
if y == nil then print(\"alias nil\") end";
    assert_eq!(run(src, "lumelir_locw_alias").trim(), "alias nil");
}

#[test]
fn regression_inline_index_still_traps() {
    // ADR 0061's inline `t[5] == nil` already works. ADR 0063
    // must not break the plain `print(t[5])` regression — that
    // still goes through the trapping read path, since the
    // widening trigger is `local x = t[i]` only.
    let src = "local t = {1}
print(t[5])";
    let out = compile_and_run(src, "lumelir_locw_inline_traps");
    assert!(
        !out.status.success(),
        "plain inline OOB read must still trap"
    );
}
