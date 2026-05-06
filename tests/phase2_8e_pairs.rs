//! Phase 2.8e-iter-pairs (ADR 0080): `for k, v in pairs(t) do BODY
//! end` — full Lua hash iteration over both array and hash parts.
//!
//! Two-phase codegen walker:
//! 1. Array phase walks 1..=length, skipping `TAG_NIL` holes (Lua
//!    spec: pairs does not stop at array holes, unlike ipairs).
//! 2. Hash phase walks 0..hash_cap, skipping `TAG_NIL` (empty)
//!    and `TAG_DELETED` (tombstone) buckets.
//!
//! **Order is unspecified** per Lua spec §3.3.5, so all hash-part
//! coverage uses sorted-line comparison via [`run_lines_sorted`].
//! Array-phase output happens to be deterministic (1..len) but we
//! still use the sort helper for symmetry — it costs nothing and
//! matches the SoT contract.
//!
//! **Rehash safety** (Codex pre-review P1): per-iteration ptr-
//! equality check on `header.hash_buf` aborts the hash phase if
//! the body grew the table. Test #14
//! (`pairs_body_grows_table_terminates_safely`) exercises this path.
//!
//! Out of scope (separate LIC entries):
//! - Generic for protocol with arbitrary callable iter —
//!   LIC-2.8e-iter-generic-1.
//! - Dynamic nil/NaN key via TaggedValue local —
//!   LIC-2.6b-hash-key-{nil,nan}-runtime-1.

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

/// Sort stdout lines so iteration-order tests don't pin Lua-
/// unspecified hash visit order. Empty lines after `trim` are
/// dropped (an empty `pairs` body produces no output at all).
fn run_lines_sorted(src: &str, output_name: &str) -> Vec<String> {
    let stdout = run(src, output_name);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = trimmed.split('\n').map(str::to_owned).collect();
    lines.sort();
    lines
}

// ============================================================
// Empty + array-only
// ============================================================

#[test]
fn pairs_empty_table_no_iterations() {
    let src = "local t = {}
for k, v in pairs(t) do print(k) end
print(\"done\")";
    assert_eq!(run(src, "lumelir_pairs_empty").trim(), "done");
}

#[test]
fn pairs_array_only_sorted() {
    let src = "local t = {10, 20, 30}
for k, v in pairs(t) do print(k, v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_arr_only"),
        vec!["1\t10", "2\t20", "3\t30"]
    );
}

#[test]
fn pairs_array_with_explicit_far_index() {
    let src = "local t = {1, 2}
t[5] = 99
for k, v in pairs(t) do print(k, v) end";
    // After `t[5]=99` the array grows so all 5 array slots exist;
    // slots 3 and 4 are TAG_NIL (Lua hole) and pairs SKIPS them.
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_arr_far_index"),
        vec!["1\t1", "2\t2", "5\t99"]
    );
}

// ============================================================
// Hash-only — one test per non-Number key kind
// ============================================================

#[test]
fn pairs_hash_string_keys_sorted() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
for k, v in pairs(t) do print(k, v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_str_keys"),
        vec!["a\t1", "b\t2", "c\t3"]
    );
}

#[test]
fn pairs_hash_bool_keys_sorted() {
    let src = "local t = {}
t[true] = 100
t[false] = 200
for k, v in pairs(t) do print(k, v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_bool_keys"),
        vec!["false\t200", "true\t100"]
    );
}

#[test]
fn pairs_hash_function_keys_sorted() {
    let src = "local f = function() return 0 end
local g = function() return 0 end
local t = {}
t[f] = 1
t[g] = 2
for k, v in pairs(t) do print(type(k), v) end";
    // Two function keys → two `function\t...` lines. We can't
    // pin ptr identities but type(k) proves the key tag survived.
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_fn_keys"),
        vec!["function\t1", "function\t2"]
    );
}

#[test]
fn pairs_hash_table_keys_sorted() {
    let src = "local u = {}
local w = {}
local t = {}
t[u] = 10
t[w] = 20
for k, v in pairs(t) do print(type(k), v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_tbl_keys"),
        vec!["table\t10", "table\t20"]
    );
}

// ============================================================
// Mixed array + hash
// ============================================================

#[test]
fn pairs_array_and_hash_mixed_sorted() {
    let src = "local t = {10, 20}
t.a = 1
t[true] = 5
for k, v in pairs(t) do print(type(k), v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_mixed"),
        vec!["boolean\t5", "number\t10", "number\t20", "string\t1"]
    );
}

// ============================================================
// Tombstone + delete + reinsert
// ============================================================

#[test]
fn pairs_after_delete_skips_tombstone() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
t.b = nil
for k, v in pairs(t) do print(k, v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_tombstone"),
        vec!["a\t1", "c\t3"]
    );
}

#[test]
fn pairs_after_delete_then_reinsert() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.a = nil
t.a = 9
for k, v in pairs(t) do print(k, v) end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_reinsert"),
        vec!["a\t9", "b\t2"]
    );
}

// ============================================================
// Break + nested + outer state
// ============================================================

#[test]
fn pairs_break_exits_loop() {
    let src = "for k, v in pairs({10, 20, 30, 40}) do
  print(v)
  if v == 20 then break end
end
print(\"done\")";
    // Array phase visits 1..len in order, so output is
    // deterministic here.
    assert_eq!(run(src, "lumelir_pairs_break").trim(), "10\n20\ndone");
}

#[test]
fn pairs_nested_outer_inner() {
    let src = "for ok, ov in pairs({10, 20}) do
  for ik, iv in pairs({3, 4}) do
    print(ov, iv)
  end
end";
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_nested"),
        vec!["10\t3", "10\t4", "20\t3", "20\t4"]
    );
}

#[test]
fn pairs_body_writes_outer_state() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
local sum = 0
for k, v in pairs(t) do sum = sum + v end
print(sum)";
    assert_eq!(run(src, "lumelir_pairs_aggregate").trim(), "6");
}

// ============================================================
// Key materialization — all 5 kinds in one table
// ============================================================

#[test]
fn pairs_key_materialization_type_dispatch() {
    let src = "local f = function() return 0 end
local u = {}
local t = {}
t[1] = \"n\"
t.s = \"s\"
t[true] = \"b\"
t[f] = \"f\"
t[u] = \"t\"
for k, v in pairs(t) do print(type(k)) end";
    // 5 kinds in a single table; sorting yields deterministic
    // output even though hash visit order is unspecified.
    assert_eq!(
        run_lines_sorted(src, "lumelir_pairs_key_kinds"),
        vec!["boolean", "function", "number", "string", "table"]
    );
}

// ============================================================
// Mutation safety (P1)
// ============================================================
//
// Phase 2.8e-iter-tk (ADR 0084) opened the natural "bump every
// value" idiom: `t[k] = v + 100` with TaggedValue key. The
// previous workaround (aggregate into a separate table) is no
// longer required; the test below now exercises the direct
// pattern as an end-to-end check that ADR 0084's IndexAssign
// dispatch keeps the iterator's hash_buf identity stable across
// in-place updates of existing keys.

#[test]
fn pairs_body_mutates_existing_value_safely() {
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
for k, v in pairs(t) do t[k] = v + 100 end
print(t.a, t.b, t.c)";
    // In-place value rewrites of *existing* hash entries don't
    // grow the hash buffer (count is unchanged), so the ADR 0080
    // per-iteration `header.hash_buf` reload sees a stable buffer
    // and iteration terminates after visiting every key once.
    assert_eq!(
        run(src, "lumelir_pairs_mutate_in_place").trim(),
        "101\t102\t103"
    );
}

#[test]
fn pairs_body_grows_table_terminates_safely() {
    // Body adds *new* keys mid-iteration. The codegen P1 guard
    // (per-iteration `header.hash_buf` reload + ptr-equality
    // check) sets `_broken` when a rehash frees the captured
    // buffer, so the loop terminates instead of dereferencing
    // freed memory. The exact iteration count is unspecified per
    // Lua spec — what we assert is (a) the binary exits 0 (no
    // crash) and (b) a sentinel keyed *outside* the iterated
    // table is reachable after the loop.
    let src = "local t = {}
t.seed = 1
for k, v in pairs(t) do t[\"k\" .. tostring(v)] = v + 1 end
print(\"survived\")
print(t.seed)";
    let out = run(src, "lumelir_pairs_grow");
    assert!(
        out.contains("survived"),
        "loop must terminate without crash, got: {out:?}"
    );
    assert!(
        out.contains("\n1\n") || out.ends_with("\n1\n") || out.contains("survived\n1"),
        "post-loop reads must succeed, got: {out:?}"
    );
}
