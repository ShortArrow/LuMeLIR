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
fn out_of_bounds_write_traps() {
    // OOB write traps via the same mechanism as 2.6a-arr's read.
    // Lua spec: silently grow / create a hole. Our static type
    // system can't represent that yet — security over Lua
    // compatibility per ADR 0054's policy.
    let src = "local t = {1, 2, 3}
t[5] = 99";
    let out = compile_and_run(src, "lumelir_26a_wr_oob");
    assert!(!out.status.success(), "OOB write must trap");
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
fn grow_write_at_length_plus_one_traps() {
    // Lua spec: t[#t+1]=v grows the array. We don't yet track
    // capacity, so this also traps. (LIC-2.6a-wr-2 — reverts
    // when capacity tracking lands.)
    let src = "local t = {1, 2, 3}
t[4] = 99";
    let out = compile_and_run(src, "lumelir_26a_wr_grow");
    assert!(!out.status.success(), "grow write must trap (for now)");
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
fn write_value_must_be_number() {
    // Heterogeneous element kinds defer until tagged values
    // arrive (LIC-2.6a-wr-3).
    let chunk = lumelir::parser::parse(
        "local t = {1, 2, 3}
t[1] = \"x\"",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn write_key_must_be_number() {
    let chunk = lumelir::parser::parse(
        "local t = {1, 2, 3}
t[\"k\"] = 99",
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
