//! Phase 2.6+-arith-metamethods (ADR 0147): per-family bundle for
//! the 8 arith metamethods (7 binary + 1 unary), Table operand(s).

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

// --- __add ---

#[test]
fn add_via_metamethod() {
    let src = r#"
local mt = {}
mt.__add = function(a, b) return 10 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x + y)
"#;
    let out = run_ok(src, "lumelir_add_meta");
    assert_eq!(out, "10\n");
}

// --- __sub ---

#[test]
fn sub_via_metamethod() {
    let src = r#"
local mt = {}
mt.__sub = function(a, b) return 11 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x - y)
"#;
    let out = run_ok(src, "lumelir_sub_meta");
    assert_eq!(out, "11\n");
}

// --- __mul ---

#[test]
fn mul_via_metamethod() {
    let src = r#"
local mt = {}
mt.__mul = function(a, b) return 12 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x * y)
"#;
    let out = run_ok(src, "lumelir_mul_meta");
    assert_eq!(out, "12\n");
}

// --- __div ---

#[test]
fn div_via_metamethod() {
    let src = r#"
local mt = {}
mt.__div = function(a, b) return 13 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x / y)
"#;
    let out = run_ok(src, "lumelir_div_meta");
    assert_eq!(out, "13\n");
}

// --- __mod ---

#[test]
fn mod_via_metamethod() {
    let src = r#"
local mt = {}
mt.__mod = function(a, b) return 14 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x % y)
"#;
    let out = run_ok(src, "lumelir_mod_meta");
    assert_eq!(out, "14\n");
}

// --- __pow ---

#[test]
fn pow_via_metamethod() {
    let src = r#"
local mt = {}
mt.__pow = function(a, b) return 15 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x ^ y)
"#;
    let out = run_ok(src, "lumelir_pow_meta");
    assert_eq!(out, "15\n");
}

// --- __idiv ---

#[test]
fn idiv_via_metamethod() {
    let src = r#"
local mt = {}
mt.__idiv = function(a, b) return 16 end
local x = setmetatable({}, mt)
local y = setmetatable({}, mt)
print(x // y)
"#;
    let out = run_ok(src, "lumelir_idiv_meta");
    assert_eq!(out, "16\n");
}

// --- __unm ---

#[test]
fn unm_via_metamethod() {
    let src = r#"
local mt = {}
mt.__unm = function(a) return 17 end
local x = setmetatable({}, mt)
print(-x)
"#;
    let out = run_ok(src, "lumelir_unm_meta");
    assert_eq!(out, "17\n");
}
