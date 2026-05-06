//! Phase 2.6b-hash-keys (ADR 0079): hash key kinds expansion.
//! Lua spec §3.4.5: any non-nil, non-NaN value can serve as a
//! table key. This phase widens the hash bucket from String-only
//! to Number / String / Bool / Function / Table keys via a tagged
//! 16-byte key slot (Plan E). Closes LIC-2.6a-arr-3 (partial).
//!
//! Out of scope (LIC entries):
//! - Dynamic nil key via TaggedValue local
//!   (LIC-2.6b-hash-key-nil-runtime-1)
//! - Dynamic NaN key via TaggedValue local
//!   (LIC-2.6b-hash-key-nan-runtime-1)
//! - `pairs(t)` hash iteration (LIC-2.8e-iter-pairs-1) — needs
//!   this phase as a prerequisite.

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

// ============================================================
// Bool keys
// ============================================================

#[test]
fn bool_key_true_insert_lookup() {
    let src = "local t = {}
t[true] = 42
print(t[true])";
    assert_eq!(run(src, "lumelir_hk_bool_true").trim(), "42");
}

#[test]
fn bool_key_false_separate_from_true() {
    let src = "local t = {}
t[true] = 1
t[false] = 2
print(t[true])
print(t[false])";
    assert_eq!(run(src, "lumelir_hk_bool_distinct").trim(), "1\n2");
}

#[test]
fn bool_key_overwrite() {
    let src = "local t = {}
t[true] = 1
t[true] = 2
print(t[true])";
    assert_eq!(run(src, "lumelir_hk_bool_overwrite").trim(), "2");
}

// ============================================================
// Function keys (raw pointer identity)
// ============================================================

#[test]
fn function_key_identity() {
    let src = "local function f() return 1 end
local t = {}
t[f] = \"fn-value\"
print(t[f])";
    assert_eq!(run(src, "lumelir_hk_fn_identity").trim(), "fn-value");
}

#[test]
fn function_key_distinct_instances() {
    let src = "local function f() return 1 end
local function g() return 2 end
local t = {}
t[f] = \"foo\"
t[g] = \"bar\"
print(t[f])
print(t[g])";
    assert_eq!(run(src, "lumelir_hk_fn_distinct").trim(), "foo\nbar");
}

// ============================================================
// Table keys (raw pointer identity)
// ============================================================

#[test]
fn table_key_identity() {
    let src = "local u = {}
local t = {}
t[u] = \"tbl-value\"
print(t[u])";
    assert_eq!(run(src, "lumelir_hk_tbl_identity").trim(), "tbl-value");
}

#[test]
fn table_key_distinct_instances() {
    let src = "local u = {}
local v = {}
local t = {}
t[u] = \"first\"
t[v] = \"second\"
print(t[u])
print(t[v])";
    assert_eq!(run(src, "lumelir_hk_tbl_distinct").trim(), "first\nsecond");
}

// ============================================================
// Cross-kind in one table
// ============================================================

#[test]
fn mixed_key_kinds_in_one_table() {
    let src = "local f = function() return 1 end
local u = {}
local t = {}
t[true] = \"bool\"
t[\"x\"] = \"str\"
t[f] = \"fn\"
t[u] = \"tbl\"
print(t[true])
print(t[\"x\"])
print(t[f])
print(t[u])";
    assert_eq!(run(src, "lumelir_hk_mixed").trim(), "bool\nstr\nfn\ntbl");
}

// ============================================================
// Delete + reinsert across kinds
// ============================================================

#[test]
fn delete_then_reinsert_bool() {
    let src = "local t = {}
t[true] = 1
t[true] = nil
t[true] = 2
print(t[true])";
    assert_eq!(run(src, "lumelir_hk_bool_delete_reinsert").trim(), "2");
}

#[test]
fn delete_function_key_then_reinsert() {
    let src = "local function f() return 1 end
local t = {}
t[f] = \"first\"
t[f] = nil
t[f] = \"second\"
print(t[f])";
    assert_eq!(run(src, "lumelir_hk_fn_delete_reinsert").trim(), "second");
}

// ============================================================
// Stress — multi-kind insertions exercising rehash
// ============================================================

#[test]
fn multi_kind_stress_rehash() {
    // 4 functions + 4 tables + 4 bool/string entries = 12,
    // crosses the load-factor 0.75 boundary at cap=8 → triggers
    // rehash to cap=16. Confirms tag-based live-test in
    // emit_hash_grow_if_needed.
    let src = "local f1 = function() return 1 end
local f2 = function() return 2 end
local f3 = function() return 3 end
local f4 = function() return 4 end
local u1 = {}
local u2 = {}
local u3 = {}
local u4 = {}
local t = {}
t[f1] = 1
t[f2] = 2
t[f3] = 3
t[f4] = 4
t[u1] = 11
t[u2] = 12
t[u3] = 13
t[u4] = 14
t[true] = 100
t[false] = 200
t[\"a\"] = 1000
t[\"b\"] = 2000
print(t[f3])
print(t[u2])
print(t[true])
print(t[\"b\"])";
    assert_eq!(run(src, "lumelir_hk_stress").trim(), "3\n12\n100\n2000");
}

// ============================================================
// Negative — Nil key still HIR-rejected
// ============================================================

#[test]
fn nil_key_literal_rejected_at_hir() {
    let chunk = lumelir::parser::parse(
        "local t = {}
t[nil] = 1",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "nil literal as hash key must remain HIR-rejected"
    );
}
