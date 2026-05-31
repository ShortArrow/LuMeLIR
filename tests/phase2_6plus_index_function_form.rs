//! Phase 2.6+-index-function-form (ADR 0150): `mt.__index = function(t, k)
//! return ... end` static-String key, Number return.

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

// --- Test 1: __index = Function dispatches on miss ---

#[test]
fn index_function_returns_computed_number() {
    let src = r#"
local mt = {}
mt.__index = function(t, k) return 100 end
local t = setmetatable({}, mt)
print(t.absent)
"#;
    let out = run_ok(src, "lumelir_idx_fn_basic");
    assert_eq!(out, "100\n");
}

// --- Test 2: present key bypasses __index entirely ---

#[test]
fn index_function_skipped_for_present_key() {
    let src = r#"
local mt = {}
mt.__index = function(t, k) return 999 end
local t = setmetatable({}, mt)
t.x = 42
print(t.x)
"#;
    let out = run_ok(src, "lumelir_idx_fn_present");
    assert_eq!(out, "42\n");
}

// --- Test 3: __index = Function chains with __newindex unaffected ---

#[test]
fn index_function_coexists_with_newindex() {
    // Regression-pin: ADR 0135's __newindex Table path must stay
    // green; the __index Function arm should not disturb it.
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
mt.__index = function(t, k) return 7 end
local t = setmetatable({}, mt)
t.w = 11
print(sink.w)
print(t.absent)
"#;
    let out = run_ok(src, "lumelir_idx_fn_coexist");
    assert_eq!(out, "11\n7\n");
}

// --- Test 4: ADR 0134 Table-form __index still works (regression pin) ---

#[test]
fn index_table_form_unchanged() {
    let src = r#"
local fallback = {}
fallback.x = 22
local mt = {}
mt.__index = fallback
local t = setmetatable({}, mt)
print(t.x)
"#;
    let out = run_ok(src, "lumelir_idx_table_form_unchanged");
    assert_eq!(out, "22\n");
}
