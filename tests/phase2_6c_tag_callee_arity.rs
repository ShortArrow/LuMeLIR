//! Phase 2.6c-tag-callee-arity (ADR 0075) → 2.5x-callee-dispatch
//! (ADR 0082): tagged-callee call hardening.
//!
//! ADR 0075 rejected every indirect call through a TaggedValue
//! local because the slot's payload was a bare function pointer
//! with no signature descriptor — `args.len()` reconstruction was
//! UB-prone on heterogeneous tables. ADR 0082 reopens the path
//! safely via per-call-site static dispatch: HIR enumerates all
//! user functions whose signature matches the call site, and
//! codegen emits a direct `func.call @user_fn_X` chain (no
//! `func.call_indirect` cast).
//!
//! The original ADR 0075 reject tests are reframed here as
//! positive coverage of the dispatch path.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(result.status.success(), "binary should exit 0: {result:?}");
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn array_indexed_function_call_dispatches() {
    // ADR 0082: `local g = t[1]; g()` resolves through the
    // dispatch chain to `f`'s direct call.
    let src = "local function f() return 42 end
local t = {f}
local g = t[1]
print(g())";
    assert_eq!(run(src, "lumelir_arity_arr_dispatch").trim(), "42");
}

#[test]
fn hash_indexed_function_call_dispatches() {
    let src = "local function f() return 7 end
local t = {}
t.f = f
local g = t.f
print(g())";
    assert_eq!(run(src, "lumelir_arity_hash_dispatch").trim(), "7");
}

#[test]
fn heterogeneous_arity_table_routes_through_dispatch_chain() {
    // The LIC-2.6c-tag-callee-arity-1 hazard case: two functions
    // with different arities in the same table. ADR 0072 reconstructed
    // a fixed signature and was UB-prone; ADR 0075 rejected the
    // path; ADR 0082 routes the call site through a static-dispatch
    // chain whose candidate set is the user functions matching the
    // *call-site*'s signature (param_kinds + ret_kinds). Calling
    // `g(1)` selects the 1-arg `f1`; `g(1, 2)` would select `f2`.
    // No UB possible.
    let src = "local function f1(x) return x end
local function f2(x, y) return x + y end
local t = {f1, f2}
local g = t[1]
print(g(7))";
    assert_eq!(run(src, "lumelir_arity_hetero_dispatch").trim(), "7");
}

#[test]
fn direct_function_call_via_known_local_still_works() {
    // Regression: a Function-kind local with a known FuncId
    // (alias of a top-level `local function`) remains callable
    // through the static-arity path ADR 0075 preserved and ADR
    // 0082 left untouched.
    let src = "local function f() return 42 end
local g = f
print(g())";
    assert_eq!(run(src, "lumelir_arity_direct").trim(), "42");
}

#[test]
fn function_parameter_indirect_call_still_works() {
    // Regression: function-parameter Function locals
    // (Phase 2.5b.2 / ADR 0018) carry a statically-known arity
    // inferred via body scan; their `Callee::Indirect` path is
    // safe and remains the call-arm of choice for the parameter
    // case (ADR 0082's `IndirectDispatch` is for TaggedValue locals
    // only).
    let src = "local function apply(g, x) return g(x) end
local function f(x) return x * 2 end
print(apply(f, 21))";
    assert_eq!(run(src, "lumelir_arity_param").trim(), "42");
}
