//! Phase 2.6+-multi-hop-number-key-newindex (ADR 0170):
//! chained Table-form `__newindex` for Number key.

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

// --- Test 1: 2-hop Table chain — t → inter → leaf ---

#[test]
fn multi_hop_number_key_newindex_two_hop_chain() {
    // t = {1,2,3}; t[5] OOB at hop 0 → routes via mt1.__newindex.
    // inter = {}; inter[5] is also OOB at hop 1 (since #inter == 0)
    // → routes via mt2.__newindex into leaf. leaf[5] = 777 lands.
    // t and inter unchanged.
    let src = r#"
local leaf = {}
local mt2 = {}
mt2.__newindex = leaf
local inter = {}
setmetatable(inter, mt2)
local mt1 = {}
mt1.__newindex = inter
local t = {1, 2, 3}
setmetatable(t, mt1)
t[5] = 777
print(#t)
print(#inter)
print(leaf[5])
print(t[5])
print(inter[5])
"#;
    let out = run_ok(src, "lumelir_multihop_newidx_basic");
    assert_eq!(out, "3\n0\n777\nnil\nnil\n");
}

// --- Test 2: ADR 0168 single-hop still works (regression pin) ---

#[test]
fn multi_hop_newindex_single_hop_unchanged() {
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local t = {1, 2, 3}
setmetatable(t, mt)
t[5] = 555
print(sink[5])
print(t[5])
"#;
    let out = run_ok(src, "lumelir_multihop_newidx_single_regression");
    assert_eq!(out, "555\nnil\n");
}
