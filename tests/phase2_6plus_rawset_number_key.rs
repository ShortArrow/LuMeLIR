//! Phase 2.6+-rawset-number-key (ADR 0172):
//! `rawset(t, n, v)` for Number key bypasses __newindex.

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

// --- Test 1: rawset(t, n, v) writes to t directly, bypassing __newindex ---

#[test]
fn rawset_number_key_bypasses_newindex() {
    // mt.__newindex = sink; t = {1,2,3}.
    // `t[5] = 50` would route to sink (ADR 0168).
    // `rawset(t, 5, 50)` must NOT route — must extend t directly.
    let src = r#"
local sink = {}
local mt = {}
mt.__newindex = sink
local t = {1, 2, 3}
setmetatable(t, mt)
rawset(t, 5, 50)
print(#t)
print(t[5])
print(sink[5])
"#;
    let out = run_ok(src, "lumelir_rawset_numkey_bypass");
    assert_eq!(out, "5\n50\nnil\n");
}

// --- Test 2: rawset returns t (Lua §6.1) ---

#[test]
fn rawset_returns_table() {
    let src = r#"
local t = {}
local r = rawset(t, 1, "x")
print(r[1])
print(t[1])
"#;
    let out = run_ok(src, "lumelir_rawset_returns_t");
    assert_eq!(out, "x\nx\n");
}

// --- Test 3: rawset Number key with mid-array TAG_NIL also bypasses ---

#[test]
fn rawset_number_key_bypasses_mid_array_nil_trigger() {
    // ADR 0171 routes mid-array TAG_NIL writes via __newindex.
    // rawset must skip that too.
    let src = r#"
local sink = {}
local t = {1}
t[3] = 3
local mt = {}
mt.__newindex = sink
setmetatable(t, mt)
rawset(t, 2, 22)
print(t[2])
print(sink[2])
"#;
    let out = run_ok(src, "lumelir_rawset_numkey_midarray");
    assert_eq!(out, "22\nnil\n");
}
