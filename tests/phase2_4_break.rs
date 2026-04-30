//! Integration test: Phase 2.4 — `break` in `while` / `for`.

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
fn while_true_break_terminates_immediately() {
    // The loop runs zero useful iterations; the program reaches the
    // post-loop print and exits successfully.
    assert_eq!(
        run(
            "while true do break end\nprint(99)",
            "lumelir_24_while_true"
        )
        .trim(),
        "99"
    );
}

#[test]
fn while_break_after_three_iterations() {
    let src = "local i = 0\nwhile i < 10 do\nif i == 3 then break end\ni = i + 1\nend\nprint(i)";
    assert_eq!(run(src, "lumelir_24_while_count").trim(), "3");
}

#[test]
fn for_break_skips_remainder() {
    // After break, neither the rest of body nor further iterations run.
    let src = "for i = 1, 5 do if i == 3 then break end\nprint(i) end";
    assert_eq!(run(src, "lumelir_24_for_skip").trim(), "1\n2");
}

#[test]
fn for_break_after_print_runs_inclusive() {
    // Print runs first, THEN break — i=3 is printed before the loop exits.
    let src = "for i = 1, 5 do print(i)\nif i == 3 then break end end";
    assert_eq!(run(src, "lumelir_24_for_inclusive").trim(), "1\n2\n3");
}

#[test]
fn nested_break_targets_innermost() {
    // Inner break only stops inner loop; outer continues.
    let src = "for i = 1, 2 do for j = 1, 5 do if j == 2 then break end end\nprint(i) end";
    assert_eq!(run(src, "lumelir_24_nested").trim(), "1\n2");
}

#[test]
fn post_break_statements_in_body_are_not_executed() {
    // 99 must NOT appear — break exits the if branch, then the next
    // body statement (print 99) is guarded out.
    let src = "for i = 1, 3 do if i == 2 then break end\nprint(99) end";
    assert_eq!(run(src, "lumelir_24_post_skip").trim(), "99");
    // i=1: if false → print(99). i=2: if true → break, post skipped.
}

#[test]
fn break_outside_loop_is_a_static_error() {
    let chunk = lumelir::parser::parse("break").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("break at top level must error");
    assert!(format!("{err}").contains("not inside any loop"));
}

#[test]
fn break_in_do_block_outside_loop_is_a_static_error() {
    let chunk = lumelir::parser::parse("do break end").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("break in bare do must error");
    assert!(format!("{err}").contains("not inside any loop"));
}
