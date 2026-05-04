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
fn hole_read_traps() {
    // The slot at index 2 was filled with Nil tag; reading it
    // into a Number-typed expression context fails the tag check.
    let src = "local t = {1}
t[3] = 3
print(t[2])";
    let out = compile_and_run(src, "lumelir_26c_hole_read");
    assert!(!out.status.success(), "hole read must trap");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        combined.contains("type mismatch") || combined.contains("table"),
        "expected type-mismatch diagnostic, got: {combined}"
    );
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
fn read_oob_still_traps() {
    // Reading past length still traps (read bound stays
    // `[1, length]`); LIC-2.6a-arr-1 unchanged.
    let src = "local t = {1}
print(t[5])";
    let out = compile_and_run(src, "lumelir_26c_read_oob");
    assert!(!out.status.success(), "OOB read must still trap");
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
