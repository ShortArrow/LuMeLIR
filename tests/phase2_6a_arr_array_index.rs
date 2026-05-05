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
fn out_of_bounds_index_prints_nil_post_2_6c_hetero_fix() {
    // ADR 0065 (Phase 2.6c-tag-hetero-fix) routes inline
    // `print(t[k])` through the non-trapping tagged path so an
    // OOB read prints "nil", matching Lua semantics. Earlier
    // ADRs (0054 / 0061) intentionally trapped on the inline
    // form; that decision is superseded under hetero values
    // because trapping on a String / Bool payload is wrong.
    let src = "local t = {1, 2, 3}
print(t[5])";
    assert_eq!(run(src, "lumelir_26a_arr_oob").trim(), "nil");
}

#[test]
fn zero_index_prints_nil_post_2_6c_hetero_fix() {
    // Lua arrays are 1-based — `t[0]` is OOB → nil.
    let src = "local t = {1, 2, 3}
print(t[0])";
    assert_eq!(run(src, "lumelir_26a_arr_zero").trim(), "nil");
}

#[test]
fn out_of_bounds_arith_use_still_traps() {
    // The trap path remains for arithmetic on a Nil-tagged
    // local — Lua spec: nil + 1 errors. The widening read is
    // non-trapping; only the arith use trips.
    let src = "local t = {1, 2, 3}
local x = t[5]
print(x + 1)";
    let out = compile_and_run(src, "lumelir_26a_arr_oob_arith");
    assert!(!out.status.success(), "nil + 1 must trap");
}

#[test]
fn function_element_is_static_error_post_2_6c_hetero() {
    // ADR 0064 (Phase 2.6c-tag-hetero) opened Bool / String /
    // Nil as valid table elements alongside Number. Function
    // elements still reject — the closure-escape / ucast path
    // is left for a follow-up sub-phase.
    let chunk = lumelir::parser::parse("local function f() return 1 end\nlocal t = {f}").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn non_arithmetic_key_kind_is_static_error_after_2_6b() {
    // Phase 2.6b-hash (ADR 0058) opened String keys as a valid
    // index kind via the hash path. Other kinds (Bool / Nil /
    // Function / Table) still reject — the only kinds we accept
    // are Number (array path) and String (hash path). See
    // `tests/phase2_6b_hash_keys.rs` for the string-key path.
    let chunk = lumelir::parser::parse(
        "local t = {1, 2, 3}
print(t[true])",
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
