//! Integration test: Phase 2.5c.3 — static rejection of closure
//! escape (ADR 0044). A Function value carrying upvalues can only
//! reach a *direct* call site (`Callee::User`); passing it as an
//! argument or returning it would route through `Callee::Indirect`
//! which has no path for upvalue threading. We catch this in HIR
//! rather than letting MLIR verification produce a cryptic
//! signature-mismatch error.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn passing_closure_with_upvalue_as_arg_is_static_error() {
    let chunk = lumelir::parser::parse(
        "local m = 10
local f = function(x) return x + m end
local function apply(g, x) return g(x) end
print(apply(f, 5))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn returning_closure_with_upvalue_is_static_error() {
    // The closure value would need to outlive its creation scope —
    // disallowed because the inner reads from the outer slot at
    // every call.
    let chunk = lumelir::parser::parse(
        "local function make()
  local m = 10
  local f = function(x) return x + m end
  return f
end
print(make()(5))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn aliasing_closure_then_passing_as_arg_is_static_error() {
    // Even via an alias, the outer slot is still the upvalue
    // source, and the alias inherits the func_id, so the check
    // fires at the call site.
    let chunk = lumelir::parser::parse(
        "local m = 10
local f = function(x) return x + m end
local g = f
local function apply(h, x) return h(x) end
print(apply(g, 5))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn direct_call_of_closure_with_upvalue_still_works() {
    // Regression: 2.5c-min direct dispatch unaffected.
    let src = "local m = 10
local f = function(x) return x + m end
print(f(5))";
    assert_eq!(run(src, "lumelir_25c3_direct").trim(), "15");
}

#[test]
fn aliasing_closure_then_calling_directly_still_works() {
    // Regression: alias preserves func_id; direct dispatch keeps
    // working through the alias.
    let src = "local m = 10
local f = function(x) return x + m end
local g = f
print(g(5))";
    assert_eq!(run(src, "lumelir_25c3_alias_direct").trim(), "15");
}

#[test]
fn passing_function_without_upvalues_as_arg_still_works() {
    // Regression: 2.5b.2 first-class function args path unaffected
    // when the function has no upvalues.
    let src = "local f = function(x) return x * 2 end
local function apply(g, x) return g(x) end
print(apply(f, 5))";
    assert_eq!(run(src, "lumelir_25c3_no_upv_arg").trim(), "10");
}

#[test]
fn returning_function_without_upvalues_still_works() {
    // Regression: 2.5b.3 function return path unaffected when no
    // upvalues are involved. Uses the `local function` form so
    // the callee is reached via function_names rather than via
    // the outer-scope capture path (which would itself reject
    // Function-kind captures per ADR 0043).
    let src = "local function d(x) return x * 2 end
local function get() return d end
local f = get()
print(f(5))";
    assert_eq!(run(src, "lumelir_25c3_no_upv_ret").trim(), "10");
}

#[test]
fn anonymous_closure_with_upvalue_inline_passed_is_static_error() {
    // Inline anonymous closure literal that captures — same
    // rejection applies because the lowered FunctionRef points to
    // a function with upvalues.
    let chunk = lumelir::parser::parse(
        "local m = 10
local function apply(g, x) return g(x) end
print(apply(function(x) return x + m end, 5))",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}
