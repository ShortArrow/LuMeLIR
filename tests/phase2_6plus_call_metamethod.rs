//! Phase 2.6+-call-metamethod (ADR 0146): `t(args)` where `t` is a
//! Table-kind Local rewrites to `t.__call(t, args)` and dispatches
//! through ADR 0091 + ADR 0082.

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

// --- Test 1: 1-arg __call (just self) ---

#[test]
fn call_table_with_self_only_dispatches() {
    let src = r#"
local mt = {}
mt.__call = function(self) return "X" end
local t = setmetatable({}, mt)
print(t())
"#;
    let out = run_ok(src, "lumelir_call_meta_self_only");
    assert_eq!(out, "X\n");
}

// --- Test 2: 2-arg __call (self + 1 extra) ---

#[test]
fn call_table_with_one_extra_arg_dispatches() {
    let src = r#"
local mt = {}
mt.__call = function(self, x) return x end
local t = setmetatable({}, mt)
print(t("hello"))
"#;
    let out = run_ok(src, "lumelir_call_meta_one_arg");
    assert_eq!(out, "hello\n");
}

// --- Test 3: __call returning Number works ---

#[test]
fn call_table_returning_number() {
    let src = r#"
local mt = {}
mt.__call = function(self, x) return x + 1 end
local t = setmetatable({}, mt)
print(t(41))
"#;
    let out = run_ok(src, "lumelir_call_meta_number");
    assert_eq!(out, "42\n");
}

// --- Test 4: Function-kind Local still dispatches via existing path ---

#[test]
fn function_local_call_still_works() {
    // Regression-pin: the new arm fires only for Table-kind Locals,
    // so existing Function-kind Local calls hit zero overhead.
    let src = r#"
local function f(x) return x + 10 end
local g = f
print(g(5))
"#;
    let out = run_ok(src, "lumelir_call_meta_fn_local");
    assert_eq!(out, "15\n");
}
