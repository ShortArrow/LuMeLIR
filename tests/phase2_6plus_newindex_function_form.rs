//! Phase 2.6+-newindex-function-form (ADR 0151): `mt.__newindex =
//! function(t, k, v) ... end` static-String key, Number value, void.

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

// --- Test 1: __newindex = Function dispatches on missing key ---

#[test]
fn newindex_function_dispatches_for_missing_key() {
    // Side effect: write value into a separate sink table keyed by
    // a fixed name (the metamethod ignores the actual key for
    // simplicity).
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = function(t, k, v) sink.stored = v end
local t = setmetatable({}, mt)
t.k = 7
print(sink.stored)
"#;
    let out = run_ok(src, "lumelir_nidx_fn_basic");
    assert_eq!(out, "7\n");
}

// --- Test 2: existing key bypasses __newindex (Lua §2.4) ---

#[test]
fn newindex_function_skipped_for_existing_key() {
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = function(t, k, v) sink.touched = 1 end
local t = {}
t.x = 10
setmetatable(t, mt)
t.x = 20
print(t.x)
if sink.touched == nil then
  print("untouched")
else
  print("touched")
end
"#;
    let out = run_ok(src, "lumelir_nidx_fn_existing");
    assert_eq!(out, "20\nuntouched\n");
}

// --- Test 3: __newindex Function coexists with __index Table ---

#[test]
fn newindex_function_coexists_with_index_table() {
    let src = r#"
local fb = {}
fb.x = 42
local sink = {}
local mt = {}
mt.__index = fb
mt.__newindex = function(t, k, v) sink.stored = v end
local t = setmetatable({}, mt)
print(t.x)
t.y = 99
print(sink.stored)
"#;
    let out = run_ok(src, "lumelir_nidx_fn_coexist");
    assert_eq!(out, "42\n99\n");
}

// --- Test 4: ADR 0135 Table-form __newindex still works ---

#[test]
fn newindex_table_form_unchanged() {
    let src = r#"
local storage = {}
local mt = {}
mt.__newindex = storage
local t = setmetatable({}, mt)
t.k = 33
print(storage.k)
"#;
    let out = run_ok(src, "lumelir_nidx_table_form_unchanged");
    assert_eq!(out, "33\n");
}
