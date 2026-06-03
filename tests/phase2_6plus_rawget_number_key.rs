//! Phase 2.6+-rawget-number-key (ADR 0173):
//! `rawget(t, n)` Number key — TaggedValue out-slot, no trap.

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

fn run_ok(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- Test 1: rawget(t, n) returns t[n] for in-range, bypasses __index ---

#[test]
fn rawget_number_key_returns_in_range_and_bypasses_index() {
    let src = r#"
local fallback = {}
fallback[5] = 999
local mt = {}
mt.__index = fallback
local t = {10, 20, 30}
setmetatable(t, mt)
print(rawget(t, 2))
print(rawget(t, 5))
print(t[5])
"#;
    let out = run_ok(src, "lumelir_rawget_numkey_bypass");
    assert_eq!(out, "20\nnil\n999\n");
}

// --- Test 2: rawget OOB returns nil, no trap ---

#[test]
fn rawget_number_key_oob_returns_nil() {
    let src = r#"
local t = {1, 2}
print(rawget(t, 10))
print(rawget(t, 0))
"#;
    let out = run_ok(src, "lumelir_rawget_numkey_oob");
    assert_eq!(out, "nil\nnil\n");
}

// --- Test 3: rawget mid-array TAG_NIL returns nil ---

#[test]
fn rawget_number_key_mid_array_nil() {
    let src = r#"
local t = {1}
t[3] = 3
print(rawget(t, 2))
print(rawget(t, 3))
"#;
    let out = run_ok(src, "lumelir_rawget_numkey_midnil");
    assert_eq!(out, "nil\n3\n");
}
