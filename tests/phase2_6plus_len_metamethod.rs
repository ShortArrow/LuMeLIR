//! Phase 2.6+-len-metamethod (ADR 0149): `#t` consults `mt.__len`
//! Function-form; falls back to raw length on every miss.

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

// --- Test 1: __len Function-form dispatches ---

#[test]
fn len_consults_metatable_len() {
    let src = r#"
local mt = {}
mt.__len = function(t) return 99 end
local t = setmetatable({1, 2, 3}, mt)
print(#t)
"#;
    let out = run_ok(src, "lumelir_len_meta_basic");
    assert_eq!(out, "99\n");
}

// --- Test 2: no metatable → raw length ---

#[test]
fn len_no_metatable_returns_raw_length() {
    let src = r#"
local t = {10, 20, 30, 40}
print(#t)
"#;
    let out = run_ok(src, "lumelir_len_no_mt");
    assert_eq!(out, "4\n");
}

// --- Test 3: metatable without __len → raw length ---

#[test]
fn len_metatable_without_field_returns_raw_length() {
    let src = r#"
local mt = {}
mt.k = 1
local t = setmetatable({1, 2, 3, 4, 5}, mt)
print(#t)
"#;
    let out = run_ok(src, "lumelir_len_mt_no_field");
    assert_eq!(out, "5\n");
}

// --- Test 4: empty Table with __len → metamethod still fires ---

#[test]
fn len_empty_table_with_metamethod() {
    let src = r#"
local mt = {}
mt.__len = function(t) return 42 end
local t = setmetatable({}, mt)
print(#t)
"#;
    let out = run_ok(src, "lumelir_len_empty_meta");
    assert_eq!(out, "42\n");
}
