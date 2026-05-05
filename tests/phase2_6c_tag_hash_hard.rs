//! Integration test: Phase 2.6c-tag-hash-hard — `t.k = nil`
//! becomes a *hard* tombstone (key replaced with sentinel,
//! probe skips past, rehash drops physically). Replaces the
//! soft-tombstone behaviour of ADR 0060. Surface behaviour
//! (read-after-delete trap, isnil-query → true) is unchanged
//! (LIC-2.6c-tag-hash-1 resolved at the structural level).

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
fn delete_nonexistent_key_is_noop() {
    let src = "local t = {}
t.k = nil
if t.k == nil then print(\"ok\") end";
    assert_eq!(run(src, "lumelir_hard_noop").trim(), "ok");
}

#[test]
fn rehash_with_deletes_sums_remaining_evens() {
    // 30 keys inserted, odd-indexed deleted via hard tombstone.
    // After multiple rehashes the sentinel slots must be
    // physically dropped so the surviving even keys still sum
    // correctly.
    let src = "local t = {}
for i = 1, 30 do
  t[\"k\" .. tostring(i)] = i
end
for i = 1, 30, 2 do
  t[\"k\" .. tostring(i)] = nil
end
print(t.k2 + t.k4 + t.k6 + t.k8 + t.k10 + t.k12 + t.k14 + t.k16 + t.k18 + t.k20 + t.k22 + t.k24 + t.k26 + t.k28 + t.k30)";
    assert_eq!(run(src, "lumelir_hard_rehash_evens").trim(), "240");
}

#[test]
fn delete_then_reinsert_repeated_no_inflation() {
    // 50 cycles of write-then-delete on the same key. Without
    // sentinel-aware probing, each cycle would either re-insert
    // at a fresh slot (inflating cap) or fail to find the
    // earlier deletion (probing past null). With hard tombstone
    // + rehash the final write should be observable.
    let src = "local t = {}
for i = 1, 50 do
  t[\"k\"] = i
  t[\"k\"] = nil
end
t[\"k\"] = 99
print(t.k)";
    assert_eq!(run(src, "lumelir_hard_repeat_noinfl").trim(), "99");
}

#[test]
fn multiple_keys_deleted_then_remainder_readable() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
t.b = nil
t.c = nil
print(t.a)";
    assert_eq!(run(src, "lumelir_hard_multidel").trim(), "1");
}

#[test]
fn delete_then_overwrite_value_correct() {
    let src = "local t = {}
t.k = 1
t.k = nil
t.k = 2
print(t.k)";
    assert_eq!(run(src, "lumelir_hard_overwrite").trim(), "2");
}

#[test]
fn alias_visibility_under_hard_delete() {
    let src = "local a = {}
local b = a
a.k = 1
a.k = nil
a.k = 99
print(b.k)";
    assert_eq!(run(src, "lumelir_hard_alias").trim(), "99");
}

#[test]
fn plain_read_after_delete_prints_nil_post_2_6c_hetero_fix() {
    // ADR 0065: inline `print(t.k)` after a hard-tombstone
    // delete prints "nil" via the tagged dispatch. The hard-
    // tombstone behaviour itself (sentinel write + rehash) is
    // unchanged.
    let src = "local t = {}
t.k = 1
t.k = nil
print(t.k)";
    assert_eq!(run(src, "lumelir_hard_read_traps").trim(), "nil");
}
