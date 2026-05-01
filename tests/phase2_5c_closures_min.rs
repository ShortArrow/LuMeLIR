//! Integration test: Phase 2.5c-min — capture-by-value closures
//! for Number locals, direct-call only (ADR 0037).

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
fn anonymous_closure_captures_number_local() {
    let src = "local x = 5
local f = function() return x end
print(f())";
    assert_eq!(run(src, "lumelir_25c_basic").trim(), "5");
}

#[test]
fn closure_captures_multiple_values() {
    let src = "local a = 3
local b = 4
local f = function() return a + b end
print(f())";
    assert_eq!(run(src, "lumelir_25c_multi").trim(), "7");
}

#[test]
fn closure_captures_param() {
    // The capture is the outer function's param, not a chunk local.
    let src = "local function outer(n)
  local f = function() return n * 2 end
  return f()
end
print(outer(7))";
    assert_eq!(run(src, "lumelir_25c_param").trim(), "14");
}

#[test]
fn closure_capture_reflects_live_binding_inside_creation_scope() {
    // Within the creation scope, the captured upvalue is the
    // outer slot — reassigning it before the call propagates,
    // matching Lua's "upvalue is the binding" semantic. The
    // "no escape" restriction (Phase 2.5c-min) makes this safe:
    // the closure cannot outlive the slot it observes.
    let src = "local x = 1
local f = function() return x end
x = 99
print(f())";
    assert_eq!(run(src, "lumelir_25c_live").trim(), "99");
}

#[test]
fn closure_uses_arg_and_capture() {
    let src = "local base = 100
local add = function(n) return base + n end
print(add(5))
print(add(20))";
    assert_eq!(run(src, "lumelir_25c_args").trim(), "105\n120");
}

#[test]
fn nested_local_function_can_capture() {
    // Phase 2.5c-min also enables capture for the nested
    // `local function` form (Phase 2.5f infrastructure).
    let src = "local function outer(x)
  local function inner()
    return x + 1
  end
  return inner()
end
print(outer(41))";
    assert_eq!(run(src, "lumelir_25c_nested").trim(), "42");
}

#[test]
fn closure_capturing_bool_is_static_error() {
    let chunk = lumelir::parser::parse(
        "local b = true
local f = function() return b end",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn closure_in_arithmetic_chain() {
    // Top-level `local function` cannot capture chunk-level locals
    // in this phase — its body is lowered in pass 2, before main
    // chunk locals exist. Use anonymous form so capture happens at
    // FunctionExpr-evaluation time (during main chunk processing).
    let src = "local m = 10
local calc = function(x) return x * m + 1 end
print(calc(3))";
    assert_eq!(run(src, "lumelir_25c_chain").trim(), "31");
}
