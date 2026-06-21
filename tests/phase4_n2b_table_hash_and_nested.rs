//! ADR 0255 — N2-B: hash-bucket walk + Tables-in-Tables.
//! Extends the N2-A propagation pass (ADR 0254) with:
//!   - Hash-bucket walk (key + value tagged slots × hash_cap).
//!   - TAG_TABLE element propagation (Tables-in-Tables).
//!   - Fixed-iteration outer loop (N=8) for transitive marking.

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn hash_field_string_value_survives_gc() {
    assert_eq!(
        run_ok(
            "local t = {}
t.name = \"world\"
collectgarbage()
print(t.name)",
            "lumelir_n2b_hash_string"
        )
        .trim(),
        "world"
    );
}

#[test]
fn hash_field_string_key_survives_gc() {
    // Several distinct hash keys plus number values. Keys are
    // String objects too and need to survive collection.
    let out = run_ok(
        "local t = {}
t.alpha = 1
t.beta = 2
t.gamma = 3
collectgarbage()
print(t.alpha)
print(t.beta)
print(t.gamma)",
        "lumelir_n2b_hash_multikey",
    );
    assert_eq!(out.trim(), "1\n2\n3");
}

#[test]
fn nested_table_in_array_survives_gc() {
    assert_eq!(
        run_ok(
            "local inner = {\"deep\"}
local outer = {inner}
collectgarbage()
print(outer[1][1])",
            "lumelir_n2b_nested_array"
        )
        .trim(),
        "deep"
    );
}

#[test]
fn nested_table_in_hash_survives_gc() {
    let out = run_ok(
        "local payload = {\"value\"}
local t = {}
t.inner = payload
collectgarbage()
print(t.inner[1])",
        "lumelir_n2b_nested_hash",
    );
    assert_eq!(out.trim(), "value");
}

#[test]
fn three_level_table_nest_survives_gc() {
    // outer.mid.deep[1] tests two levels of Tables-in-Tables.
    let out = run_ok(
        "local deep = {\"bottom\"}
local mid = {deep}
local outer = {mid}
collectgarbage()
print(outer[1][1][1])",
        "lumelir_n2b_three_levels",
    );
    assert_eq!(out.trim(), "bottom");
}

#[test]
fn mixed_array_and_hash_table_survives_gc() {
    let out = run_ok(
        "local t = {\"first\", \"second\"}
t.label = \"my-table\"
collectgarbage()
print(t[1])
print(t[2])
print(t.label)",
        "lumelir_n2b_mixed",
    );
    assert_eq!(out.trim(), "first\nsecond\nmy-table");
}

#[test]
fn unreferenced_nested_string_outside_kept_tree_is_freed() {
    // A transient string-objects allocation not reachable
    // through any chunk root should be freed.
    let out = run_ok(
        "local kept = {\"alive\"}
local transient = tostring(7)
local freed = collectgarbage()
print(kept[1])
if freed > 0 then print(\"freed\") else print(\"zero\") end",
        "lumelir_n2b_transient_outside",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "alive");
    assert_eq!(lines[1], "freed");
}
