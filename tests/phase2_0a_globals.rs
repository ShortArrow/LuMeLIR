//! Integration test: Phase 2.0a — auto-declare globals at the
//! chunk top level (ADR 0048). A bare `name = expr` statement at
//! top level (no enclosing function body) implicitly declares
//! `name` as a chunk-level local; subsequent reads/writes use it.

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
fn bare_assignment_at_top_level_declares_global() {
    assert_eq!(run("PI = 3\nprint(PI)", "lumelir_20a_basic").trim(), "3");
}

#[test]
fn bare_string_global_works() {
    assert_eq!(
        run("greeting = \"hi\"\nprint(greeting)", "lumelir_20a_str").trim(),
        "hi"
    );
}

#[test]
fn bare_bool_global_works() {
    assert_eq!(
        run(
            "flag = true\nif flag then print(1) else print(2) end",
            "lumelir_20a_bool"
        )
        .trim(),
        "1"
    );
}

#[test]
fn global_reassignment_keeps_kind() {
    let src = "n = 1
n = 99
print(n)";
    assert_eq!(run(src, "lumelir_20a_reassign").trim(), "99");
}

#[test]
fn global_kind_change_is_static_error() {
    let chunk = lumelir::parser::parse("n = 1\nn = \"hi\"\n").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn global_captured_by_top_level_function() {
    // The auto-declared global is a chunk-level local, so 2.5c.1's
    // top-level capture path picks it up automatically.
    let src = "PI = 3
local function area(r) return PI * r * r end
print(area(5))";
    assert_eq!(run(src, "lumelir_20a_capture").trim(), "75");
}

#[test]
fn bare_assignment_inside_function_body_is_static_error() {
    // ADR 0048 limits auto-declare to top level; inside a
    // function body, an unresolved name on the LHS still errors.
    let chunk = lumelir::parser::parse(
        "local function f()
  x = 1
end
f()",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn bare_assignment_inside_block_still_resolves_to_chunk_global() {
    // A `do ... end` block at top level is still considered "top
    // level" — assignments there auto-declare at chunk scope.
    let src = "do
  x = 5
end
print(x)";
    assert_eq!(run(src, "lumelir_20a_doblock").trim(), "5");
}

#[test]
fn cannot_shadow_function_name_via_global() {
    // The name `helper` is registered in `function_names`; bare
    // `helper = 1` would create an ambiguity. Reject statically.
    let chunk = lumelir::parser::parse(
        "local function helper() return 1 end
helper = 99",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn local_keyword_still_works_as_before() {
    // Regression: `local x = 1` is unchanged.
    let src = "local x = 7
print(x)";
    assert_eq!(run(src, "lumelir_20a_local_regression").trim(), "7");
}
