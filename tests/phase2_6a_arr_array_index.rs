//! Integration test: Phase 2.6a-arr — Number-only array literal
//! `{1, 2, 3}` and integer indexing read `t[i]` (ADR 0054).
//! Out-of-bounds reads trap with `exit(1)`.

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
fn array_literal_length_three() {
    let src = "local t = {1, 2, 3}
print(#t)";
    assert_eq!(run(src, "lumelir_26a_arr_len").trim(), "3");
}

#[test]
fn read_first_element() {
    let src = "local t = {10, 20, 30}
print(t[1])";
    assert_eq!(run(src, "lumelir_26a_arr_first").trim(), "10");
}

#[test]
fn read_middle_element() {
    let src = "local t = {10, 20, 30}
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_arr_mid").trim(), "20");
}

#[test]
fn read_last_element() {
    let src = "local t = {10, 20, 30}
print(t[3])";
    assert_eq!(run(src, "lumelir_26a_arr_last").trim(), "30");
}

#[test]
fn elements_can_be_summed() {
    let src = "local t = {10, 20, 30}
print(t[1] + t[2] + t[3])";
    assert_eq!(run(src, "lumelir_26a_arr_sum").trim(), "60");
}

#[test]
fn empty_table_still_compiles_after_2_6a_arr() {
    // Regression: 2.6a-min `{}` form still works.
    let src = "local t = {}
print(#t)";
    assert_eq!(run(src, "lumelir_26a_arr_empty").trim(), "0");
}

#[test]
fn trailing_comma_in_array_literal_is_allowed() {
    let src = "local t = {1, 2, 3,}
print(#t)";
    assert_eq!(run(src, "lumelir_26a_arr_trailing").trim(), "3");
}

#[test]
fn direct_indexing_of_table_literal() {
    // `{10, 20, 30}[2]` — Lua's prefix-expression rule lets a
    // table constructor be indexed directly.
    let src = "print(({10, 20, 30})[2])";
    assert_eq!(run(src, "lumelir_26a_arr_direct").trim(), "20");
}

#[test]
fn array_with_computed_elements() {
    let src = "local n = 5
local t = {n, n * 2, n * 3}
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_arr_computed").trim(), "10");
}

#[test]
fn array_indexed_by_local_variable() {
    let src = "local t = {100, 200, 300}
local i = 2
print(t[i])";
    assert_eq!(run(src, "lumelir_26a_arr_var_idx").trim(), "200");
}

#[test]
fn out_of_bounds_index_traps() {
    // OOB read exits with `exit(1)` — the compiler emits a
    // bounds check on every index op (ADR 0054). This is a
    // diversion from Lua's "return nil"; until heterogeneous
    // returns arrive, security wins over compatibility.
    let src = "local t = {1, 2, 3}
print(t[5])";
    let out = compile_and_run(src, "lumelir_26a_arr_oob");
    assert!(!out.status.success(), "OOB read must trap");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("table index out of bounds") || combined.contains("out of bounds"),
        "expected OOB diagnostic, got: {combined}"
    );
}

#[test]
fn zero_index_traps() {
    // Lua arrays are 1-based — `t[0]` is OOB.
    let src = "local t = {1, 2, 3}
print(t[0])";
    let out = compile_and_run(src, "lumelir_26a_arr_zero");
    assert!(!out.status.success(), "t[0] must trap");
}

#[test]
fn non_number_element_is_static_error() {
    // 2.6a-arr restricts elements to Number — heterogeneous
    // arrays land later (when tagged values arrive).
    let chunk = lumelir::parser::parse("local t = {1, \"two\", 3}").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn non_number_key_is_static_error() {
    // String keys belong to the hash part (2.6b); Number keys only.
    let chunk = lumelir::parser::parse(
        "local t = {1, 2, 3}
print(t[\"one\"])",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn indexing_a_number_is_static_error() {
    // `local n = 5; n[1]` — only Tables are indexable.
    let chunk = lumelir::parser::parse(
        "local n = 5
print(n[1])",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
