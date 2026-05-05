//! Integration test: Phase 2.6c-tag-arr — tagged array slots
//! enabling `t[#t+2] = v` hole writes (ADR 0059). Slots become
//! `{i64 tag, f64 value}` 16 bytes; Number/Nil tags only this
//! phase. LIC-2.6a-wr-1 resolved.

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
fn basic_hole_write() {
    let src = "local t = {}
t[1] = 1
t[3] = 3
print(t[3])";
    assert_eq!(run(src, "lumelir_26c_basic").trim(), "3");
}

#[test]
fn length_grows_past_hole() {
    let src = "local t = {}
t[3] = 99
print(#t)";
    assert_eq!(run(src, "lumelir_26c_len").trim(), "3");
}

#[test]
fn hole_read_prints_nil_post_2_6c_hetero_fix() {
    // ADR 0065: inline `print(t[k])` dispatches on the slot
    // tag; a Nil-tagged hole prints "nil" per Lua spec. Earlier
    // ADR 0059 trapped on the tag mismatch; that decision is
    // superseded under the heterogeneous-aware print path.
    let src = "local t = {1}
t[3] = 3
print(t[2])";
    assert_eq!(run(src, "lumelir_26c_hole_read").trim(), "nil");
}

#[test]
fn hole_arith_still_traps() {
    // The trap path remains for arithmetic on a Nil-tagged
    // local — Lua spec: nil + 1 errors.
    let src = "local t = {1}
t[3] = 3
local x = t[2]
print(x + 1)";
    let out = compile_and_run(src, "lumelir_26c_hole_arith");
    assert!(!out.status.success(), "hole arith use must trap");
}

#[test]
fn large_hole() {
    let src = "local t = {1, 2}
t[10] = 99
print(t[10])
print(#t)";
    assert_eq!(run(src, "lumelir_26c_large_hole"), "99\n10\n");
}

#[test]
fn fill_all_holes_then_read() {
    let src = "local t = {}
t[5] = 5
t[1] = 1
t[2] = 2
t[3] = 3
t[4] = 4
print(t[1] + t[2] + t[3] + t[4] + t[5])";
    assert_eq!(run(src, "lumelir_26c_fill").trim(), "15");
}

#[test]
fn alias_visibility_under_hole_write() {
    // ADR 0056's stable header keeps both aliases pointing at the
    // same header; the hole-fill + length update happens through
    // the shared header.
    let src = "local a = {1, 2}
local b = a
a[5] = 5
print(b[5])
print(#b)";
    assert_eq!(run(src, "lumelir_26c_alias"), "5\n5\n");
}

#[test]
fn hole_plus_grow_stress() {
    // Sparse fill of 30 odd indices forces multiple realloc +
    // gap-fill cycles. Tests that the new 16-byte slot layout
    // and gap-fill loop interact correctly under doubling.
    let src = "local t = {}
for i = 1, 30, 2 do
  t[i] = i
end
print(t[15])
print(#t)";
    assert_eq!(run(src, "lumelir_26c_stress"), "15\n29\n");
}

#[test]
fn zero_index_still_traps() {
    // The lower bound (`key < 1`) check is unchanged.
    let src = "local t = {}
t[0] = 99";
    let out = compile_and_run(src, "lumelir_26c_zero");
    assert!(!out.status.success(), "t[0] = v must still trap");
}

#[test]
fn read_oob_prints_nil_post_2_6c_hetero_fix() {
    // ADR 0065: OOB inline `print(t[k])` prints "nil" via the
    // tagged dispatch path; arith on the Nil-tagged result
    // would still trap.
    let src = "local t = {1}
print(t[5])";
    assert_eq!(run(src, "lumelir_26c_read_oob").trim(), "nil");
}

#[test]
fn array_construction_regression() {
    // `{1, 2, 3}` builds a 3-slot Number array.
    let src = "local t = {10, 20, 30}
print(t[1] + t[2] + t[3])";
    assert_eq!(run(src, "lumelir_26c_construction").trim(), "60");
}

#[test]
fn hash_path_unaffected_regression() {
    // hash_buf is unchanged this phase — string keys keep
    // working without tag dispatch.
    let src = "local t = {}
t.x = 99
t.y = 1
print(t.x + t.y)";
    assert_eq!(run(src, "lumelir_26c_hash_regression").trim(), "100");
}
