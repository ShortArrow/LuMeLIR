//! Integration test: Phase 2.6a-wr — Number-only array element
//! write `t[i] = v` (ADR 0055). Mirror of 2.6a-arr's read path —
//! same bounds-check trap on OOB.

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
fn write_then_read_existing_index() {
    let src = "local t = {1, 2, 3}
t[2] = 99
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_wr_basic").trim(), "99");
}

#[test]
fn write_via_self_reference_in_rhs() {
    let src = "local t = {1, 2, 3}
t[1] = t[2] + t[3]
print(t[1])";
    assert_eq!(run(src, "lumelir_26a_wr_self").trim(), "5");
}

#[test]
fn loop_writes_each_element() {
    let src = "local t = {0, 0, 0}
for i = 1, 3 do
  t[i] = i * 10
end
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_wr_loop").trim(), "20");
}

#[test]
fn out_of_bounds_write_now_creates_hole_after_2_6c_tag_arr() {
    // Phase 2.6c-tag-arr (ADR 0059) lifted the upper-bound trap.
    // `t[5] = 99` on a length-3 table now creates Nil-tagged
    // holes at indices 4 and 5-1=4 (i.e. just slot 4), then sets
    // t[5] and extends length to 5. The lower-bound (`key < 1`)
    // trap is unchanged — see `zero_index_write_traps` below.
    let src = "local t = {1, 2, 3}
t[5] = 99
print(t[5])
print(#t)";
    let out = compile_and_run(src, "lumelir_26a_wr_oob");
    assert!(out.status.success(), "hole creation must succeed");
    assert_eq!(String::from_utf8_lossy(&out.stdout).into_owned(), "99\n5\n",);
}

#[test]
fn grow_write_at_length_plus_one_now_works_after_2_6a_grow() {
    // Phase 2.6a-grow (ADR 0057) resolved LIC-2.6a-wr-2. The
    // one-past-end push slot is now a valid write target — it
    // grows the array and updates length. Coverage of the grow
    // path itself lives in `tests/phase2_6a_grow_array_push.rs`.
    let src = "local t = {1, 2, 3}
t[4] = 99
print(t[4])
print(#t)";
    let out = compile_and_run(src, "lumelir_26a_wr_grow");
    assert!(out.status.success(), "grow write must succeed now");
    assert_eq!(String::from_utf8_lossy(&out.stdout).into_owned(), "99\n4\n",);
}

#[test]
fn zero_index_write_traps() {
    let src = "local t = {1, 2, 3}
t[0] = 99";
    let out = compile_and_run(src, "lumelir_26a_wr_zero");
    assert!(!out.status.success(), "t[0] = v must trap");
}

#[test]
fn alias_write_is_visible_through_original() {
    // Tables are reference values: assigning `local b = a`
    // copies the heap pointer, so a write through `b` is
    // observable through `a`.
    let src = "local a = {1, 2, 3}
local b = a
b[1] = 99
print(a[1])";
    assert_eq!(run(src, "lumelir_26a_wr_alias").trim(), "99");
}

#[test]
fn write_closure_with_upvalue_still_rejects_post_2_6c_tag_fn_tbl() {
    // ADR 0071 (Phase 2.6c-tag-fn-tbl) accepts Number / Bool /
    // String / Function (closure-less) / Table values for
    // `t[i] = v`. Closures with upvalues still reject
    // (LIC-2.6c-tag-hetero-closure-escape-1) via the existing
    // ClosureEscapes analysis (ADR 0044, extended in ADR 0071).
    let chunk = lumelir::parser::parse(
        "local x = 1
local f = function() return x end
local t = {1, 2, 3}
t[1] = f",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn write_key_must_be_arithmetic_or_string_after_2_6b() {
    // Phase 2.6b-hash (ADR 0058) opened String keys via the hash
    // path. Bool/Nil/Function/Table keys still reject. See
    // `tests/phase2_6b_hash_keys.rs` for the string-key write
    // path coverage.
    let chunk = lumelir::parser::parse(
        "local t = {1, 2, 3}
t[true] = 99",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn write_target_must_be_table() {
    let chunk = lumelir::parser::parse(
        "local n = 5
n[1] = 99",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn read_path_still_works_after_2_6a_wr() {
    // Regression: 2.6a-arr's read path is unaffected.
    let src = "local t = {10, 20, 30}
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_wr_read_regression").trim(), "20");
}
