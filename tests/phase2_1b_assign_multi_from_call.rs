//! Integration test: Phase 2.1b — multi-target reassignment from
//! a multi-result Call: `a, b = pair()` (ADR 0050). Symmetric to
//! Phase 2.5d's `local a, b = pair()`.

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
fn assign_multi_from_call_to_existing_locals() {
    let src = "local function pair() return 10, 20 end
local a = 0
local b = 0
a, b = pair()
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_21b_basic"), "10\n20\n");
}

#[test]
fn assign_multi_from_call_to_globals_auto_declares() {
    // ADR 0048 + ADR 0050: bare-name targets at chunk scope auto-
    // declare; the multi-result Call fills both.
    let src = "local function pair() return 7, 11 end
x, y = pair()
print(x)
print(y)";
    assert_eq!(run(src, "lumelir_21b_globals"), "7\n11\n");
}

#[test]
fn assign_multi_from_call_arity_mismatch_is_static_error() {
    // Callee returns 2, but targets are 3.
    let chunk = lumelir::parser::parse(
        "local function pair() return 1, 2 end
local a = 0
local b = 0
local c = 0
a, b, c = pair()",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assign_multi_from_builtin_call_rejects() {
    // `print` is a builtin (no statically-tracked ret_kinds); it
    // can't be the source of a multi-target reassignment.
    let chunk = lumelir::parser::parse(
        "local a = 0
local b = 0
a, b = print(1)",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assign_multi_from_call_kind_mismatch_target_is_static_error() {
    // Existing local kind doesn't match the call's ret_kind at
    // that position. (Currently both pair returns are Number;
    // but the targets include a String-kind local.)
    let chunk = lumelir::parser::parse(
        "local function pair() return 1, 2 end
local a = 0
local b = \"x\"
a, b = pair()",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn assign_multi_from_call_chained_use() {
    // The reassigned values feed into subsequent code.
    let src = "local function divmod(a, b) return a, a end
local q = 0
local r = 0
q, r = divmod(7, 3)
print(q + r)";
    assert_eq!(run(src, "lumelir_21b_chain").trim(), "14");
}

#[test]
fn parallel_assign_still_works_after_2_1b() {
    // Regression: parallel (2.1a) form unaffected.
    let src = "local a = 1
local b = 2
a, b = b, a
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_21b_parallel_regression"), "2\n1\n");
}
