//! Phase 2.6+-bitwise-metamethods (ADR 0148): 6 bitwise
//! metamethods (5 binary + 1 unary) for Table operand(s).

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

// --- __band ---

#[test]
fn band_via_metamethod() {
    let src = r#"
local mt = {}
mt.__band = function(a, b) return 20 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x & y)
"#;
    let out = run_ok(src, "lumelir_band_meta");
    assert_eq!(out, "20\n");
}

// --- __bor ---

#[test]
fn bor_via_metamethod() {
    let src = r#"
local mt = {}
mt.__bor = function(a, b) return 21 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x | y)
"#;
    let out = run_ok(src, "lumelir_bor_meta");
    assert_eq!(out, "21\n");
}

// --- __bxor ---

#[test]
fn bxor_via_metamethod() {
    let src = r#"
local mt = {}
mt.__bxor = function(a, b) return 22 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x ~ y)
"#;
    let out = run_ok(src, "lumelir_bxor_meta");
    assert_eq!(out, "22\n");
}

// --- __shl ---

#[test]
fn shl_via_metamethod() {
    let src = r#"
local mt = {}
mt.__shl = function(a, b) return 23 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x << y)
"#;
    let out = run_ok(src, "lumelir_shl_meta");
    assert_eq!(out, "23\n");
}

// --- __shr ---

#[test]
fn shr_via_metamethod() {
    let src = r#"
local mt = {}
mt.__shr = function(a, b) return 24 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x >> y)
"#;
    let out = run_ok(src, "lumelir_shr_meta");
    assert_eq!(out, "24\n");
}

// --- __bnot ---

#[test]
fn bnot_via_metamethod() {
    let src = r#"
local mt = {}
mt.__bnot = function(a) return 25 end
local x = setmetatable({}, mt)
print(~x)
"#;
    let out = run_ok(src, "lumelir_bnot_meta");
    assert_eq!(out, "25\n");
}
