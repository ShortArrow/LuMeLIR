//! Integration test: Phase 2.5b.2 — passing functions as arguments
//! via `func.call_indirect` (ADR 0018).

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
fn apply_pattern_with_one_arg_function() {
    let src = "local function apply(g, x) return g(x) end\nlocal f = function(x) return x*2 end\nprint(apply(f, 7))";
    assert_eq!(run(src, "lumelir_25b2_apply").trim(), "14");
}

#[test]
fn compose_pattern_double_then_inc() {
    let src = "local function compose(f, g, x) return f(g(x)) end\nlocal d = function(x) return x*2 end\nlocal i = function(x) return x+1 end\nprint(compose(d, i, 5))";
    assert_eq!(run(src, "lumelir_25b2_compose").trim(), "12");
}

#[test]
fn direct_call_still_works_after_2_5b2_changes() {
    // Regression check: Phase 2.5b's direct-call fast path must still
    // function (when callee has a known FuncId).
    let src = "local f = function(x) return x*3 end\nprint(f(4))";
    assert_eq!(run(src, "lumelir_25b2_direct").trim(), "12");
}

#[test]
fn arity_mismatch_on_function_arg_is_static_error() {
    // f has arity 2, but apply expects g with arity 1 (inferred from
    // `g(x)`). Passing f to apply must fail to lower.
    let chunk = lumelir::parser::parse(
        "local function apply(g, x) return g(x) end\nlocal f = function(a, b) return a+b end\napply(f, 5)",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn zero_arg_function_passed_to_zero_arg_caller() {
    let src = "local function call0(g) return g() end\nlocal f = function() return 99 end\nprint(call0(f))";
    assert_eq!(run(src, "lumelir_25b2_zero").trim(), "99");
}

#[test]
fn two_arg_function_passed_via_param() {
    let src = "local function call2(g, a, b) return g(a, b) end\nlocal f = function(a, b) return a+b end\nprint(call2(f, 3, 4))";
    assert_eq!(run(src, "lumelir_25b2_two").trim(), "7");
}

#[test]
fn higher_order_nested_application() {
    let src = "local function apply(g, x) return g(x) end\nlocal inc = function(x) return x+1 end\nlocal double = function(x) return x*2 end\nprint(apply(double, apply(inc, 5)))";
    assert_eq!(run(src, "lumelir_25b2_nested").trim(), "12");
}

#[test]
fn alias_then_pass_as_arg() {
    // `local g = f` then `apply(g, ...)` — the alias still resolves.
    let src = "local f = function(x) return x*10 end\nlocal g = f\nlocal function apply(h, x) return h(x) end\nprint(apply(g, 4))";
    assert_eq!(run(src, "lumelir_25b2_alias_arg").trim(), "40");
}
