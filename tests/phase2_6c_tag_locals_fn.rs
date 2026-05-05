//! Integration test: Phase 2.6c-tag-locals-fn (ADR 0074) —
//! function-return TaggedValue widening. When a Lua function
//! returns nil on one path and a typed value (Number / Bool /
//! String / Table / Function) on another, the function's return
//! kind widens to `TaggedValue` so callers can store the value
//! into a heterogeneous local. Closes
//! `LIC-2.6c-tag-locals-fn-1`.
//!
//! Out of scope (separate LIC entries):
//! - Indirect call (`Callee::Indirect`) returning TaggedValue —
//!   the call-site arity reconstruction has no static signature
//!   to consult (LIC-2.6c-tag-locals-fn-indirect-1).
//! - Multi-return × TaggedValue interleaving — `return 1, nil`
//!   vs `return nil, 1` mixing across positions
//!   (LIC-2.6c-tag-locals-fn-multi-1).
//! - Tagged-callee arity hardening (LIC-2.6c-tag-callee-arity-1).

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

fn run(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0, got {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ============================================================
// Direct call: Number / Nil widening
// ============================================================

#[test]
fn function_returning_number_or_nil_widens_to_tagged_local() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return nil
end
local v = maybe(-1)
print(v)";
    assert_eq!(run(src, "lumelir_widen_num_nil_neg").trim(), "nil");
}

#[test]
fn function_returning_number_or_nil_returns_value_path() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return nil
end
local v = maybe(1)
print(v)";
    assert_eq!(run(src, "lumelir_widen_num_nil_pos").trim(), "42");
}

#[test]
fn function_returning_number_or_nil_consumed_by_type() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return nil
end
print(type(maybe(-1)))
print(type(maybe(1)))";
    assert_eq!(run(src, "lumelir_widen_type").trim(), "nil\nnumber");
}

#[test]
fn function_returning_number_or_nil_consumed_by_tostring() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return nil
end
print(tostring(maybe(-1)))
print(tostring(maybe(1)))";
    assert_eq!(run(src, "lumelir_widen_tostring").trim(), "nil\n42");
}

#[test]
fn function_returning_number_or_nil_eq_nil_check() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return nil
end
local v = maybe(-1)
if v == nil then print(\"nil-branch\") else print(\"value-branch\") end
local w = maybe(1)
if w == nil then print(\"nil-branch\") else print(\"value-branch\") end";
    assert_eq!(
        run(src, "lumelir_widen_eq_nil").trim(),
        "nil-branch\nvalue-branch"
    );
}

// ============================================================
// Direct call: Bool / Nil widening
// ============================================================

#[test]
fn function_returning_bool_or_nil_widens() {
    let src = "local function maybe(x)
  if x > 0 then return true end
  return nil
end
print(type(maybe(1)))
print(type(maybe(-1)))";
    assert_eq!(run(src, "lumelir_widen_bool_nil").trim(), "boolean\nnil");
}

// ============================================================
// Direct call: String / Nil widening
// ============================================================

#[test]
fn function_returning_string_or_nil_widens() {
    let src = "local function maybe(x)
  if x > 0 then return \"yes\" end
  return nil
end
print(maybe(1))
print(maybe(-1))";
    assert_eq!(run(src, "lumelir_widen_str_nil").trim(), "yes\nnil");
}

// ============================================================
// Direct call: Number / Bool widening (non-Nil heterogeneous)
// ============================================================

#[test]
fn function_returning_number_or_bool_widens() {
    let src = "local function maybe(x)
  if x > 0 then return 42 end
  return true
end
print(tostring(maybe(1)))
print(tostring(maybe(-1)))";
    assert_eq!(run(src, "lumelir_widen_num_bool").trim(), "42\ntrue");
}

// ============================================================
// Negative — indirect call rejection (LIC-2.6c-tag-locals-fn-indirect-1)
// ============================================================

#[test]
fn tagged_return_function_called_through_tagged_local_rejected_at_hir() {
    // A function whose return widens to TaggedValue cannot be
    // invoked through Callee::Indirect today — the call-site
    // arity reconstruction has no signature info to build the
    // 2-result return type.
    let chunk = lumelir::parser::parse(
        "local function maybe(x) if x > 0 then return 42 end return nil end
local t = {maybe}
local g = t[1]
print(g(1))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "calling a TaggedValue-returning function via a tagged-slot local must be HIR-rejected"
    );
}

// ============================================================
// Regression — pure-Number return ABI unchanged
// ============================================================

#[test]
fn regression_pure_number_return_unchanged() {
    let src = "local function f() return 42 end
print(f())";
    assert_eq!(run(src, "lumelir_widen_reg_num").trim(), "42");
}

#[test]
fn regression_pure_nil_return_unchanged() {
    let src = "local function f() return nil end
print(f())";
    assert_eq!(run(src, "lumelir_widen_reg_nil").trim(), "nil");
}
