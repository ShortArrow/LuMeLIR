//! Integration test: Phase 2.6c-isnil-query — `t[i] == nil` and
//! `t.k == nil` (and `~=` variants) lowered to a non-trapping
//! IsNilQuery (ADR 0061). Inline comparisons against nil report
//! Lua-spec true/false without trapping; plain reads still trap.

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
fn array_oob_returns_true() {
    let src = "local t = {1, 2, 3}
if t[5] == nil then print(\"oob\") end";
    assert_eq!(run(src, "lumelir_isnil_oob").trim(), "oob");
}

#[test]
fn array_in_bounds_number_returns_false() {
    let src = "local t = {1, 2, 3}
if t[1] == nil then print(\"y\") else print(\"n\") end";
    assert_eq!(run(src, "lumelir_isnil_inbounds").trim(), "n");
}

#[test]
fn array_hole_returns_true() {
    let src = "local t = {}
t[3] = 3
if t[2] == nil then print(\"hole\") end";
    assert_eq!(run(src, "lumelir_isnil_hole").trim(), "hole");
}

#[test]
fn hash_missing_key_returns_true() {
    let src = "local t = {}
if t.x == nil then print(\"missing\") end";
    assert_eq!(run(src, "lumelir_isnil_missing").trim(), "missing");
}

#[test]
fn hash_present_key_with_ne_returns_true() {
    let src = "local t = {}
t.x = 5
if t.x ~= nil then print(\"present\") end";
    assert_eq!(run(src, "lumelir_isnil_present").trim(), "present");
}

#[test]
fn hash_deleted_key_returns_true() {
    let src = "local t = {}
t.x = 1
t.x = nil
if t.x == nil then print(\"deleted\") end";
    assert_eq!(run(src, "lumelir_isnil_deleted").trim(), "deleted");
}

#[test]
fn array_negative_index_returns_true() {
    let src = "local t = {1, 2, 3}
if t[0] == nil then print(\"zero\") end";
    assert_eq!(run(src, "lumelir_isnil_zero").trim(), "zero");
}

#[test]
fn combined_and_with_present_keys() {
    let src = "local t = {1, 2}
t.k = 99
if t[1] ~= nil and t.k ~= nil then print(\"both\") end";
    assert_eq!(run(src, "lumelir_isnil_combined").trim(), "both");
}

#[test]
fn empty_hash_table_query() {
    let src = "local t = {}
if t.anything == nil then print(\"empty\") end";
    assert_eq!(run(src, "lumelir_isnil_empty_hash").trim(), "empty");
}

#[test]
fn reverse_order_nil_eq_index() {
    let src = "local t = {}
if nil == t.x then print(\"yes\") end";
    assert_eq!(run(src, "lumelir_isnil_reverse").trim(), "yes");
}

#[test]
fn plain_read_still_traps_regression() {
    // The IsNilQuery pattern only activates when paired with `nil`
    // in a BinOp::Eq/Ne. Plain `print(t[oob])` keeps trapping —
    // separate code path, regression coverage.
    let src = "local t = {1}
print(t[5])";
    let out = compile_and_run(src, "lumelir_isnil_plain_regression");
    assert!(!out.status.success(), "plain OOB read must still trap");
}
