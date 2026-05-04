//! Integration test: Phase 2.6c-tag-hash — tagged hash entry
//! values + `t.k = nil` soft-delete (ADR 0060). Hash entries
//! grow to 24 bytes (`{ptr key, tagged value slot}`); writing
//! Nil marks the entry as deleted while keeping the key for
//! re-insertion.

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
fn write_delete_rewrite_cycle() {
    let src = "local t = {}
t.k = 1
t.k = nil
t.k = 2
print(t.k)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_cycle").trim(), "2");
}

#[test]
fn delete_preserves_other_keys() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.b = nil
print(t.a)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_preserve").trim(), "1");
}

#[test]
fn delete_then_read_traps() {
    let src = "local t = {}
t.k = 1
t.k = nil
print(t.k)";
    let out = compile_and_run(src, "lumelir_26c_tag_hash_read_after_del");
    assert!(!out.status.success(), "delete-then-read must trap");
}

#[test]
fn dot_syntax_delete_then_bracket_read() {
    let src = "local t = {}
t[\"k\"] = 1
t.k = nil
t.k = 2
print(t[\"k\"])";
    assert_eq!(run(src, "lumelir_26c_tag_hash_dot_bracket").trim(), "2");
}

#[test]
fn multiple_deletes_chain() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
t.b = nil
t.c = nil
print(t.a)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_multi_del").trim(), "1");
}

#[test]
fn rehash_with_deletes() {
    // Insert 30 keys, delete every other, then re-insert. Forces
    // rehash with mixed live/Nil-tagged entries surviving.
    let src = "local t = {}
for i = 1, 30 do
  t[\"k\" .. tostring(i)] = i
end
for i = 1, 30, 2 do
  t[\"k\" .. tostring(i)] = nil
end
print(t.k2 + t.k4 + t.k6 + t.k8 + t.k10)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_rehash_del").trim(), "30");
}

#[test]
fn alias_visibility_under_delete() {
    // Stable header: `local b = a` shares the same hash_buf via
    // the header. Delete + re-write through `a` is observed
    // through `b`.
    let src = "local a = {}
local b = a
a.k = 1
a.k = nil
a.k = 99
print(b.k)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_alias").trim(), "99");
}

#[test]
fn hash_value_string_now_accepted_post_2_6c_hetero() {
    // ADR 0064 (Phase 2.6c-tag-hetero) now accepts String hash
    // values. The parse + lower pipeline must succeed.
    let chunk = lumelir::parser::parse(
        "local t = {}
t.k = \"hello\"",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_ok());
}

#[test]
fn array_path_unchanged_regression() {
    // 2.6c-tag-arr's tagged array slots and hole-write keep
    // working — hash layout change is independent of array_buf.
    let src = "local t = {}
t[5] = 99
print(t[5])
print(#t)";
    assert_eq!(run(src, "lumelir_26c_tag_hash_arr_regression"), "99\n5\n");
}

#[test]
fn hash_insert_only_regression() {
    // No-delete hash insert keeps working with the new 24-byte
    // entry layout.
    let src = "local t = {}
t.x = 100
t.y = 200
print(t.x + t.y)";
    assert_eq!(
        run(src, "lumelir_26c_tag_hash_insert_regression").trim(),
        "300"
    );
}

#[test]
fn hash_30_key_stress_regression() {
    // 30-key insert without deletes; rehash still works at the
    // new entry size.
    let src = "local t = {}
for i = 1, 30 do
  t[\"k\" .. tostring(i)] = i * i
end
print(t.k15)";
    assert_eq!(
        run(src, "lumelir_26c_tag_hash_stress_regression").trim(),
        "225"
    );
}
