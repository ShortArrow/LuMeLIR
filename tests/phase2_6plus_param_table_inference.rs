//! Phase 2.6+ param-table-context-inference (ADR 0180):
//! HIR infers a parameter as `ValueKind::Table` when the body
//! uses it in a Table context (pairs/ipairs/Index/MethodCall/
//! Table-consumer builtin).

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

// --- Test 1: pairs(param) inside function body ---

#[test]
fn param_used_with_pairs_is_inferred_table() {
    let src = r#"
local function first_val(t)
  for k, v in pairs(t) do
    return v
  end
end
local x = {}
x.foo = 42
print(first_val(x))
"#;
    let out = run_ok(src, "lumelir_param_pairs");
    assert_eq!(out, "42\n");
}

// --- Test 2: ipairs(param) inside function body ---

#[test]
fn param_used_with_ipairs_is_inferred_table() {
    let src = r#"
local function sum_array(t)
  local s = 0
  for i, v in ipairs(t) do
    s = s + v
  end
  return s
end
local arr = {10, 20, 30}
print(sum_array(arr))
"#;
    let out = run_ok(src, "lumelir_param_ipairs");
    assert_eq!(out, "60\n");
}

// --- Test 3: param[k] Index access ---

#[test]
fn param_used_with_index_is_inferred_table() {
    let src = r#"
local function pick_foo(t)
  return t.foo
end
local x = {}
x.foo = 7
print(pick_foo(x))
"#;
    let out = run_ok(src, "lumelir_param_index");
    assert_eq!(out, "7\n");
}

// --- Test 4: rawget(param, k) — Table-consumer builtin first-arg ---

#[test]
fn param_used_with_rawget_is_inferred_table() {
    let src = r#"
local function raw_pick(t)
  return rawget(t, "foo")
end
local x = {}
x.foo = 99
print(raw_pick(x))
"#;
    let out = run_ok(src, "lumelir_param_rawget");
    assert_eq!(out, "99\n");
}
