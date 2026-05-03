//! Integration test: Phase 2.1a — multi-target reassignment
//! `a, b = e1, e2` (ADR 0049). Lua's parallel-evaluation rule
//! makes the swap idiom work without temporaries.

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
fn parallel_assign_two_locals() {
    let src = "local a = 1
local b = 2
a, b = 10, 20
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_21a_basic"), "10\n20\n");
}

#[test]
fn swap_via_parallel_assign() {
    // The classic Lua idiom — parallel evaluation reads both
    // RHS values *before* either assignment.
    let src = "local a = 1
local b = 2
a, b = b, a
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_21a_swap"), "2\n1\n");
}

#[test]
fn three_way_rotation() {
    let src = "local a = 1
local b = 2
local c = 3
a, b, c = c, a, b
print(a)
print(b)
print(c)";
    assert_eq!(run(src, "lumelir_21a_rot"), "3\n1\n2\n");
}

#[test]
fn parallel_assign_to_globals() {
    let src = "x = 0
y = 0
x, y = 7, 11
print(x)
print(y)";
    assert_eq!(run(src, "lumelir_21a_globals"), "7\n11\n");
}

#[test]
fn parallel_assign_arity_mismatch_is_static_error() {
    // `a, b = 1, 2, 3` has more values than targets — reject.
    // Lua silently truncates; we error instead, matching the
    // single-kind-per-slot strictness our compiler already
    // imposes elsewhere.
    let chunk = lumelir::parser::parse(
        "local a = 0
local b = 0
a, b = 1, 2, 3",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn parallel_assign_target_kind_mismatch_is_static_error() {
    // `a` is Number, `b` is String — assigning a String to `a`
    // rejects.
    let chunk = lumelir::parser::parse(
        "local a = 1
local b = \"x\"
a, b = b, a",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn parallel_assign_unknown_target_inside_function_errors() {
    // Inside a function, the unresolved-name path errors per
    // ADR 0048 — multi-target re-assign inherits the same rule.
    let chunk = lumelir::parser::parse(
        "local function f()
  a, b = 1, 2
end
f()",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn parallel_assign_string_swap() {
    let src = "local s = \"first\"
local t = \"second\"
s, t = t, s
print(s)
print(t)";
    assert_eq!(run(src, "lumelir_21a_str"), "second\nfirst\n");
}

#[test]
fn local_multi_decl_still_works_as_regression() {
    // Phase 2.5d's `local a, b = 1, 2` form is unaffected.
    let src = "local a, b = 100, 200
print(a)
print(b)";
    assert_eq!(run(src, "lumelir_21a_localmulti"), "100\n200\n");
}
