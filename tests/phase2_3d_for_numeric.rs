//! Integration test: Phase 2.3d — numeric `for` loops.

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
fn forward_count_implicit_step() {
    let out = run("for i=1,3 do print(i) end", "lumelir_23d_fwd");
    assert_eq!(out, "1\n2\n3\n");
}

#[test]
fn reverse_count_negative_step() {
    let out = run("for i=10,1,-2 do print(i) end", "lumelir_23d_rev");
    assert_eq!(out, "10\n8\n6\n4\n2\n");
}

#[test]
fn forward_count_step_two() {
    let out = run("for i=1,5,2 do print(i) end", "lumelir_23d_step2");
    assert_eq!(out, "1\n3\n5\n");
}

#[test]
fn out_of_range_skips_body() {
    // start > stop with default step 1 → loop never executes.
    let out = run("for i=1,0 do print(i) end", "lumelir_23d_skip");
    assert_eq!(out, "");
}

#[test]
fn equal_bounds_runs_once() {
    let out = run("for i=5,5 do print(i) end", "lumelir_23d_one");
    assert_eq!(out, "5\n");
}

#[test]
fn for_body_can_mutate_outer_local() {
    // Sum 1..3 into an outer local — proves loop body sees outer scope.
    let src = "local s = 0\nfor i=1,3 do s = s + i end\nprint(s)";
    assert_eq!(run(src, "lumelir_23d_sum").trim(), "6");
}

#[test]
fn nested_for_walks_2x2_grid() {
    // Outer i = 1..2, inner j = 1..2; print i*10 + j to make order visible.
    let src = "for i=1,2 do for j=1,2 do print(i*10 + j) end end";
    let out = run(src, "lumelir_23d_nested");
    assert_eq!(out, "11\n12\n21\n22\n");
}

#[test]
fn loop_var_is_invisible_after_for() {
    // `print(i)` after the loop must fail to lower (UndefinedName) —
    // we observe this through the parse+lower pipeline directly.
    let chunk = lumelir::parser::parse("for i=1,3 do end\nprint(i)").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("loop var must not leak");
    let msg = format!("{err}");
    assert!(msg.contains("'i'"), "got: {msg}");
}
