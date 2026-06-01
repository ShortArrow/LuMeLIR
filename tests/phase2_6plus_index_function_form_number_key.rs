//! Phase 2.6+-index-function-form-number-key (ADR 0166): `t[i]`
//! array-OOB → `mt.__index = function(t, k) ... end` dispatch.

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

// --- Test 1: __index Function-form returns computed Number for Number-key OOB ---

#[test]
fn index_fn_number_key_returns_computed_value() {
    // Function returns k * 10. ADR 0166 introduces this fallback:
    //   t = {1,2,3}; t[5] hits OOB; mt.__index = function(t, k);
    //   dispatched as f(t, 5.0), returns 50.
    let src = r#"
local function f(t, k) return k * 10 end
f({}, 1.0)
local mt = {}
mt.__index = f
local t = {1, 2, 3}
setmetatable(t, mt)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_idx_fn_numkey_basic");
    assert_eq!(out, "50\n");
}

// --- Test 2: ADR 0165 Table-form Number-key __index still works (regression) ---

#[test]
fn index_table_form_number_key_unchanged() {
    let src = r#"
local fallback = {}
fallback[5] = 555
local t = {1, 2, 3}
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_idx_table_numkey_regression");
    assert_eq!(out, "555\n");
}
