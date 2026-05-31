//! Phase 2.6+-metatable-field-hiding (ADR 0140):
//! `__metatable` protection field semantics per Lua §6.1.
//!
//! Red Day 0 entry: written before `emit_getmetatable_runtime` and
//! `emit_setmetatable_runtime` grow the `__metatable` probe.
//! Goes Green in C3.

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

// --- Test 1: getmetatable returns __metatable field value (String) ---

#[test]
fn getmetatable_returns_metatable_field_value() {
    let src = r#"
local mt = {}
mt.__metatable = "protected"
local t = {}
setmetatable(t, mt)
print(getmetatable(t))
"#;
    let out = run_ok(src, "lumelir_metatable_field_hide_get");
    assert_eq!(out, "protected\n");
}

// --- Test 2: setmetatable on protected mt traps ---

#[test]
fn setmetatable_on_protected_traps() {
    let src = r#"
local mt = {}
mt.__metatable = "locked"
local t = {}
setmetatable(t, mt)
local mt2 = {}
setmetatable(t, mt2)
"#;
    let out = compile_and_run(src, "lumelir_metatable_field_setm_trap");
    assert!(
        !out.status.success(),
        "setmetatable on a protected table must trap, but binary exited 0: {out:?}"
    );
}

// --- Test 3: setmetatable(t, nil) on protected also traps ---

#[test]
fn setmetatable_nil_on_protected_traps() {
    let src = r#"
local mt = {}
mt.__metatable = "locked"
local t = {}
setmetatable(t, mt)
setmetatable(t, nil)
"#;
    let out = compile_and_run(src, "lumelir_metatable_field_clear_trap");
    assert!(
        !out.status.success(),
        "setmetatable(t, nil) on a protected table must trap, but binary exited 0: {out:?}"
    );
}

// --- Test 4: rawget bypasses __metatable on getmetatable ---

#[test]
fn rawget_does_not_protect_metatable_storage() {
    // rawget on the metatable table directly retrieves the
    // __metatable field's raw value. The protection only fires via
    // getmetatable / setmetatable on the protected target.
    let src = r#"
local mt = {}
mt.__metatable = "hidden"
print(rawget(mt, "__metatable"))
"#;
    let out = run_ok(src, "lumelir_metatable_field_rawget");
    assert_eq!(out, "hidden\n");
}

// --- Test 5: no __metatable field → getmetatable returns mt as Table ---

#[test]
fn getmetatable_without_field_returns_metatable() {
    let src = r#"
local mt = {}
mt.k = 1
local t = {}
setmetatable(t, mt)
print(type(getmetatable(t)))
"#;
    let out = run_ok(src, "lumelir_metatable_field_unprotected");
    assert_eq!(out, "table\n");
}
