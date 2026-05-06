//! Phase 2.8e-iter-next (ADR 0081): direct-use tests for the
//! `next(t, prev_k)` builtin. Lua spec §3.7.3 stateless hash-
//! iteration step. Both return positions are TaggedValue; the
//! test surface uses `local k, v = next(...)` multi-assign exclusively
//! (single-value `next` is HIR-rejected today — see ADR 0081).
//!
//! Indirectly, the same code path is exercised by every test in
//! `tests/phase2_8e_pairs.rs` once ADR 0081 commit 3 lands the
//! ForPairs HIR-desugar via `next`.

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

#[test]
fn next_empty_table_returns_nil_pair() {
    let src = "local t = {}
local k, v = next(t, nil)
print(k, v)";
    assert_eq!(run(src, "lumelir_next_empty").trim(), "nil\tnil");
}

#[test]
fn next_array_only_traversal() {
    // Sequential next on a 3-element array. Order is deterministic
    // for the array part (1..len), so exact equality is fine here.
    let src = "local t = {10, 20, 30}
local k1, v1 = next(t, nil)
local k2, v2 = next(t, k1)
local k3, v3 = next(t, k2)
local k4, v4 = next(t, k3)
print(k1, v1)
print(k2, v2)
print(k3, v3)
print(k4, v4)";
    assert_eq!(
        run(src, "lumelir_next_array").trim(),
        "1\t10\n2\t20\n3\t30\nnil\tnil"
    );
}

#[test]
fn next_hash_only_sorted_traversal() {
    // 3 hash keys; visit order is unspecified per Lua spec, so we
    // compare sorted line output.
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
local k1, v1 = next(t, nil)
local k2, v2 = next(t, k1)
local k3, v3 = next(t, k2)
local k4, v4 = next(t, k3)
print(k1, v1)
print(k2, v2)
print(k3, v3)
print(k4, v4)";
    assert_eq!(
        run_lines_sorted(src, "lumelir_next_hash"),
        vec!["a\t1", "b\t2", "c\t3", "nil\tnil"]
    );
}

#[test]
fn next_array_then_hash_transition() {
    // Mixed: 2 array + 2 hash. After the array part is exhausted,
    // next should transition into the hash part.
    let src = "local t = {10, 20}
t.a = 100
t.b = 200
local k1, v1 = next(t, nil)
local k2, v2 = next(t, k1)
local k3, v3 = next(t, k2)
local k4, v4 = next(t, k3)
local k5, v5 = next(t, k4)
print(k1, v1)
print(k2, v2)
print(k3, v3)
print(k4, v4)
print(k5, v5)";
    let out = run(src, "lumelir_next_transition");
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines.len(), 5);
    // Array part is deterministic 1..len.
    assert_eq!(lines[0], "1\t10");
    assert_eq!(lines[1], "2\t20");
    // Hash part order is unspecified; both `a/b` lines must appear.
    let mut hash_lines: Vec<&str> = lines[2..4].to_vec();
    hash_lines.sort();
    assert_eq!(hash_lines, vec!["a\t100", "b\t200"]);
    assert_eq!(lines[4], "nil\tnil");
}

#[test]
fn next_skips_tombstones() {
    // Delete one key; next must skip the TAG_DELETED bucket and
    // visit only the remaining live entries.
    let src = "local t = {}
t.a = 1
t.b = 2
t.c = 3
t.b = nil
local k1, v1 = next(t, nil)
local k2, v2 = next(t, k1)
local k3, v3 = next(t, k2)
print(k1, v1)
print(k2, v2)
print(k3, v3)";
    assert_eq!(
        run_lines_sorted(src, "lumelir_next_tombstone"),
        vec!["a\t1", "c\t3", "nil\tnil"]
    );
}
