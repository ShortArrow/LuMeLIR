//! Phase 2.6+-setmetatable-nil-clear (ADR 0138):
//! `setmetatable(t, nil)` clears `t`'s metatable.
//!
//! Red Day 0 entry: written before HIR widens
//! `Builtin::SetMetatable` arg-1 accepted kinds to `{Table, Nil}`.
//! Goes Green in C3 when HIR + codegen Nil branch lands.

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

// --- Test 1: clear restores raw read semantics ---

#[test]
fn setmetatable_nil_restores_raw_read() {
    let src = r#"
local fallback = {}
fallback.x = 42
local mt = {}
mt.__index = fallback
local t = {}
setmetatable(t, mt)
print(t.x)
setmetatable(t, nil)
local after = rawget(t, "x")
if after == nil then
  print("cleared")
else
  print("still_present")
end
"#;
    let out = run_ok(src, "lumelir_setmetatable_nil_clear_read");
    assert_eq!(out, "42\ncleared\n");
}

// --- Test 2: clear returns t (Lua spec §6.1) ---

#[test]
fn setmetatable_nil_returns_table() {
    let src = r#"
local t = {}
t.k = "v"
local r = setmetatable(t, nil)
r.other = "w"
print(t.other)
print(t.k)
"#;
    let out = run_ok(src, "lumelir_setmetatable_nil_return");
    assert_eq!(out, "w\nv\n");
}

// --- Test 3: clear then install fresh mt works ---

#[test]
fn setmetatable_nil_then_install_fresh() {
    let src = r#"
local sink1 = {}
local sink2 = {}
local mt1 = {}
mt1.__newindex = sink1
local mt2 = {}
mt2.__newindex = sink2
local t = {}
setmetatable(t, mt1)
t.first = 1
setmetatable(t, nil)
setmetatable(t, mt2)
t.second = 2
print(sink1.first)
print(sink2.second)
if sink1.second == nil then
  print("sink1_clean")
end
if sink2.first == nil then
  print("sink2_clean")
end
"#;
    let out = run_ok(src, "lumelir_setmetatable_nil_reinstall");
    assert_eq!(out, "1\n2\nsink1_clean\nsink2_clean\n");
}
