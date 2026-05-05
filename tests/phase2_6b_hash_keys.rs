//! Integration test: Phase 2.6b-hash — string-keyed field access
//! `t.k` / `t["k"]` (ADR 0058). Open addressing with linear
//! probing on top of stable header's `hash_buf` (offset 24).
//! Number-only values for now (LIC-2.6b-hash-2).

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
fn write_then_read_string_key_bracket_form() {
    let src = "local t = {}
t[\"k\"] = 99
print(t[\"k\"])";
    assert_eq!(run(src, "lumelir_26b_basic").trim(), "99");
}

#[test]
fn dot_syntax_read() {
    let src = "local t = {}
t[\"k\"] = 99
print(t.k)";
    assert_eq!(run(src, "lumelir_26b_dot_read").trim(), "99");
}

#[test]
fn dot_syntax_write() {
    let src = "local t = {}
t.k = 99
print(t[\"k\"])";
    assert_eq!(run(src, "lumelir_26b_dot_write").trim(), "99");
}

#[test]
fn multiple_string_keys() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
print(t.a + t.b + t.c)";
    assert_eq!(run(src, "lumelir_26b_multi").trim(), "6");
}

#[test]
fn overwrite_same_key() {
    let src = "local t = {}
t.x = 1
t.x = 99
print(t.x)";
    assert_eq!(run(src, "lumelir_26b_overwrite").trim(), "99");
}

#[test]
fn stress_rehash_30_keys() {
    // 30 keys force at least one rehash from initial cap=8
    // (load factor 0.75 trips at count=6 → grow to 16; then at
    // count=12 → grow to 32). Ensures rehash code path works.
    let src = "local t = {}
for i = 1, 30 do
  t[\"key\" .. tostring(i)] = i * i
end
print(t.key15)
print(t.key1)
print(t.key30)";
    let out = run(src, "lumelir_26b_stress");
    assert_eq!(out, "225\n1\n900\n");
}

#[test]
fn array_and_hash_coexist() {
    // Same table holds both array elements (1-indexed numeric)
    // and hash entries (string keys). They live in independent
    // buffers reached via the stable header.
    let src = "local t = {1, 2, 3}
t.x = 99
print(t[1] + t.x)";
    assert_eq!(run(src, "lumelir_26b_coexist").trim(), "100");
}

#[test]
fn length_op_ignores_hash_part() {
    // `#t` is the length of the array part only — Lua spec.
    let src = "local t = {1, 2}
t.k = 99
print(#t)";
    assert_eq!(run(src, "lumelir_26b_len").trim(), "2");
}

#[test]
fn missing_key_prints_nil_post_2_6c_hetero_fix() {
    // ADR 0065 (Phase 2.6c-tag-hetero-fix): inline `print(t.k)`
    // dispatches at runtime on the slot tag; missing keys
    // print "nil" per Lua spec. The earlier trap (LIC-2.6b-hash-1)
    // is now resolved at this surface.
    let src = "local t = {}
print(t.missing)";
    assert_eq!(run(src, "lumelir_26b_missing").trim(), "nil");
}

#[test]
fn alias_visibility_hash_write() {
    // Stable header (ADR 0056) means `local b = a` shares the
    // same header — writes through `a` to the hash part are
    // visible through `b`.
    let src = "local a = {}
local b = a
a.k = 99
print(b.k)";
    assert_eq!(run(src, "lumelir_26b_alias").trim(), "99");
}

#[test]
fn number_key_still_works_regression() {
    let src = "local t = {1, 2, 3}
print(t[2])";
    assert_eq!(run(src, "lumelir_26b_num_regression").trim(), "2");
}

#[test]
fn hash_value_closure_with_upvalue_still_rejects_post_2_6c_tag_fn_tbl() {
    // ADR 0071 (Phase 2.6c-tag-fn-tbl) accepts closure-less
    // Function and Table hash values alongside Number / Bool /
    // String / Nil-delete. Closures with upvalues still reject
    // (LIC-2.6c-tag-hetero-closure-escape-1).
    let chunk = lumelir::parser::parse(
        "local x = 1
local f = function() return x end
local t = {}
t.k = f",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn hash_only_table_length_zero() {
    let src = "local t = {}
t.a = 1
t.b = 2
print(#t)";
    assert_eq!(run(src, "lumelir_26b_hashonly_len").trim(), "0");
}
