//! Phase 2.6+-multi-hop-number-key-index (ADR 0167):
//! chained Table-form `__index` for Number key.

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

// --- Test 1: 2-hop Table chain — t → mt1 → mt2[5] = 999 ---

#[test]
fn multi_hop_number_key_index_two_hop_chain() {
    // t = {1, 2, 3}; t[5] OOB at hop 0.
    // mt1.__index = inter (inter[5] OOB too).
    // mt2.__index = leaf (leaf[5] = 999).
    // setmetatable(inter, mt2); setmetatable(t, mt1).
    // Expected: t[5] → 999.
    let src = r#"
local leaf = {}
leaf[5] = 999
local mt2 = {}
mt2.__index = leaf
local inter = {}
setmetatable(inter, mt2)
local mt1 = {}
mt1.__index = inter
local t = {1, 2, 3}
setmetatable(t, mt1)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_multihop_numkey_basic");
    assert_eq!(out, "999\n");
}

// --- Test 2: ADR 0165 single-hop still works (regression pin) ---

#[test]
fn multi_hop_single_hop_unchanged() {
    let src = r#"
local fallback = {}
fallback[5] = 555
local t = {1, 2, 3}
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_multihop_numkey_single_regression");
    assert_eq!(out, "555\n");
}
