//! Phase 2.6+-newindex-number-key-table-form (ADR 0168):
//! `t[i] = v` (key > length) routes to `mt.__newindex` Table.

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

// --- Test 1: t[i] = v with i > length AND mt.__newindex Table → routes ---

#[test]
fn newindex_number_key_routes_to_inner_table() {
    // t has length 3; t[5] = 777 should NOT grow t; it should land
    // in sink[5]. Outer t's length stays 3, t[5] reads nil (no
    // entry); sink[5] reads 777.
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local t = {1, 2, 3}
setmetatable(t, mt)
t[5] = 777
print(#t)
print(sink[5])
print(t[5])
"#;
    let out = run_ok(src, "lumelir_newidx_numkey_route");
    assert_eq!(out, "3\n777\nnil\n");
}

// --- Test 2: plain array-grow without __newindex still works (regression) ---

#[test]
fn newindex_number_key_no_metatable_unchanged() {
    let src = r#"
local t = {1, 2, 3}
t[5] = 999
print(#t)
print(t[5])
"#;
    let out = run_ok(src, "lumelir_newidx_numkey_no_mt");
    assert_eq!(out, "5\n999\n");
}
