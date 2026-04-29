//! Integration test: Phase 2.1 — reassignment + `do ... end` block scopes.

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> String {
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
fn compile_and_run_reassignment_prints_latest_value() {
    let stdout = compile_and_run("local x = 1\nx = 2\nprint(x)", "lumelir_phase2_1_assign");
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn compile_and_run_block_shadow_then_restore() {
    // Inner `local x = 99` shadows the outer `x` within the block.
    // After the block, the outer `x = 2` (set by reassignment) must surface.
    let src = "local x = 1\nx = 2\ndo local x = 99\nprint(x) end\nprint(x)";
    let stdout = compile_and_run(src, "lumelir_phase2_1_block_shadow");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["99", "2"], "got {stdout:?}");
}

#[test]
fn compile_and_run_assignment_inside_block_escapes_block() {
    // Assigning inside a `do ... end` to an outer local persists outside.
    let src = "local x = 1\ndo x = 42 end\nprint(x)";
    let stdout = compile_and_run(src, "lumelir_phase2_1_block_assign_outer");
    assert_eq!(stdout.trim(), "42");
}
