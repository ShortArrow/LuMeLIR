//! Integration test: Phase 2.5e — Bool/Nil parameters and return
//! values for user-defined functions (ADR 0020).

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
fn bool_returning_predicate_via_comparison() {
    let src = "local function pos(x) return x > 0 end
print(pos(5))
print(pos(-3))";
    assert_eq!(run(src, "lumelir_25e_pos").trim(), "true\nfalse");
}

#[test]
fn bool_param_inferred_from_call_site() {
    // The call site `negate(true)` infers `b` as Bool.
    let src = "local function negate(b) return not b end
print(negate(true))
print(negate(false))";
    assert_eq!(run(src, "lumelir_25e_negate").trim(), "false\ntrue");
}

#[test]
fn nil_returning_function_prints_nil() {
    let src = "local function n() return nil end
print(n())";
    assert_eq!(run(src, "lumelir_25e_nil_ret").trim(), "nil");
}

#[test]
fn bool_predicate_stored_in_local_then_printed() {
    let src = "local function is_zero(n) return n == 0 end
local b = is_zero(0)
print(b)";
    assert_eq!(run(src, "lumelir_25e_is_zero").trim(), "true");
}

#[test]
fn bool_predicate_used_in_if_condition() {
    let src = "local function pos(x) return x > 0 end
if pos(5) then print(1) else print(0) end";
    assert_eq!(run(src, "lumelir_25e_pred_if").trim(), "1");
}

#[test]
fn arg_kind_mismatch_after_first_call_is_static_error() {
    // First call site `f(true)` infers `x` as Bool; the second call
    // `f(1)` then fails the param-vs-arg type check.
    let chunk = lumelir::parser::parse(
        "local function f(x) return x end
f(true)
f(1)",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn number_signatures_unchanged_after_2_5e() {
    // Regression: existing Number-only signatures still lower and
    // produce identical observable output.
    let src = "local function sq(x) return x * x end
print(sq(7))";
    assert_eq!(run(src, "lumelir_25e_regress").trim(), "49");
}
