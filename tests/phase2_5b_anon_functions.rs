//! Integration test: Phase 2.5b — anonymous function expressions and
//! first-class function values resolved at HIR time (ADR 0017).

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
fn anonymous_function_no_args_returns_constant() {
    let src = "local f = function() return 1 end\nprint(f())";
    assert_eq!(run(src, "lumelir_25b_const").trim(), "1");
}

#[test]
fn anonymous_function_doubles_argument() {
    let src = "local f = function(x) return x * 2 end\nprint(f(7))";
    assert_eq!(run(src, "lumelir_25b_double").trim(), "14");
}

#[test]
fn anonymous_function_two_args() {
    let src = "local f = function(a, b) return a + b end\nprint(f(2, 3))";
    assert_eq!(run(src, "lumelir_25b_add").trim(), "5");
}

#[test]
fn alias_via_local_assignment_works() {
    let src = "local f = function() return 42 end\nlocal g = f\nprint(g())";
    assert_eq!(run(src, "lumelir_25b_alias").trim(), "42");
}

#[test]
fn three_step_alias_chain_resolves() {
    let src = "local f = function(x) return x + 1 end\nlocal g = f\nlocal h = g\nprint(h(5))";
    assert_eq!(run(src, "lumelir_25b_chain").trim(), "6");
}

#[test]
fn function_used_as_print_arg_is_static_error() {
    let chunk = lumelir::parser::parse("local f = function() end\nprint(f)").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("print(f) must reject");
    assert!(format!("{err}").contains("function"));
}

#[test]
fn function_passed_as_user_arg_is_static_error() {
    // apply expects a Number; passing f (Function-kind) must fail to
    // lower. The exact error variant depends on resolution order, but
    // it must NOT be a successful lower.
    let chunk = lumelir::parser::parse(
        "local f = function() return 1 end\nlocal function apply(g) return g end\napply(f)",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn local_function_aliased_via_function_expr_local() {
    // Mix the two function-introduction forms: a `local function`
    // (Phase 2.5a) referenced by name from a `local g = f` (Phase 2.5b).
    let src = "local function f(x) return x * 3 end\nlocal g = f\nprint(g(4))";
    assert_eq!(run(src, "lumelir_25b_mix").trim(), "12");
}
