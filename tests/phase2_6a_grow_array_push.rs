//! Integration test: Phase 2.6a-grow — `t[#t+1] = v` push back
//! with capacity-doubling realloc (ADR 0057). Stable header from
//! ADR 0056 keeps aliases valid across the resize.

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
fn empty_table_push_then_read() {
    let src = "local t = {}
t[1] = 99
print(t[1])";
    assert_eq!(run(src, "lumelir_26a_grow_basic").trim(), "99");
}

#[test]
fn empty_table_push_grows_length() {
    let src = "local t = {}
t[1] = 99
print(#t)";
    assert_eq!(run(src, "lumelir_26a_grow_len").trim(), "1");
}

#[test]
fn multi_push_from_empty() {
    let src = "local t = {}
t[1] = 1
t[2] = 2
t[3] = 3
print(t[1] + t[2] + t[3])";
    assert_eq!(run(src, "lumelir_26a_grow_multi").trim(), "6");
}

#[test]
fn push_to_existing_array() {
    let src = "local t = {10, 20}
t[3] = 99
print(t[3])";
    assert_eq!(run(src, "lumelir_26a_grow_existing").trim(), "99");
}

#[test]
fn loop_push_into_empty() {
    let src = "local t = {}
for i = 1, 5 do
  t[i] = i * i
end
print(t[3])";
    assert_eq!(run(src, "lumelir_26a_grow_loop").trim(), "9");
}

#[test]
fn loop_push_grows_length() {
    let src = "local t = {}
for i = 1, 5 do
  t[i] = i
end
print(#t)";
    assert_eq!(run(src, "lumelir_26a_grow_loop_len").trim(), "5");
}

#[test]
fn stress_doubling_30_pushes() {
    // Tests that the doubling capacity pattern holds beyond a few
    // reallocs. 30 elements forces multiple grows: 1 → 2 → 4 → 8
    // → 16 → 32, exercising the realloc + memcpy path repeatedly.
    let src = "local t = {}
for i = 1, 30 do
  t[i] = i * 10
end
print(t[15])
print(#t)";
    let out = run(src, "lumelir_26a_grow_stress");
    assert_eq!(out, "150\n30\n");
}

#[test]
fn alias_visibility_under_grow() {
    // ★ The key test that validates ADR 0056's stable header
    // contract. `local b = a` copies the header pointer; a grow
    // through `a` reallocates `array_buf` *inside* the header, so
    // `b` (still pointing at the same header) sees the new
    // elements transparently.
    let src = "local a = {}
local b = a
for i = 1, 5 do
  a[i] = i
end
print(b[3])
print(#b)";
    let out = run(src, "lumelir_26a_grow_alias");
    assert_eq!(out, "3\n5\n");
}

#[test]
fn hole_creation_now_works_after_2_6c_tag_arr() {
    // Phase 2.6c-tag-arr (ADR 0059) resolved LIC-2.6a-wr-1.
    // `t[5] = 99` on a length-2 table now fills indices 3 and 4
    // with Nil-tagged slots and extends length to 5. Reading the
    // hole indices still traps (tag mismatch). Coverage of the
    // hole-write path itself lives in
    // `tests/phase2_6c_tag_arr_holes.rs`.
    let src = "local t = {1, 2}
t[5] = 99
print(t[5])
print(#t)";
    let out = compile_and_run(src, "lumelir_26a_grow_hole");
    assert!(out.status.success(), "hole creation must succeed now");
    assert_eq!(String::from_utf8_lossy(&out.stdout).into_owned(), "99\n5\n",);
}

#[test]
fn in_place_write_regression_after_grow() {
    // Existing 2.6a-wr in-bounds write path still works.
    let src = "local t = {1, 2, 3}
t[2] = 99
print(t[2])";
    assert_eq!(run(src, "lumelir_26a_grow_inplace").trim(), "99");
}

#[test]
fn read_oob_prints_nil_after_grow_post_2_6c_hetero_fix() {
    // ADR 0065: inline `print(t[oob])` returns "nil" via the
    // tagged dispatch path. Grow-side semantics (push at
    // `length + 1`) are unchanged.
    let src = "local t = {1, 2}
print(t[5])";
    assert_eq!(run(src, "lumelir_26a_grow_oob_read").trim(), "nil");
}
