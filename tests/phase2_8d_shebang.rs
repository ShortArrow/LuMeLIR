//! Integration test: Phase 2.8d — `#!` shebang line skip
//! (ADR 0041). A leading `#!` at byte 0 is dropped so scripts
//! marked executable on Unix still parse.

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
fn shebang_then_print_compiles() {
    let src = "#!/usr/bin/env lumelir\nprint(42)";
    assert_eq!(run(src, "lumelir_28d_basic").trim(), "42");
}

#[test]
fn shebang_with_arguments_then_program() {
    let src = "#!/usr/bin/env -S lumelir compile --foo\nprint(\"hi\")";
    assert_eq!(run(src, "lumelir_28d_args").trim(), "hi");
}

#[test]
fn shebang_followed_by_local_function() {
    let src = "#!/usr/bin/env lumelir\nlocal function add(a, b) return a + b end\nprint(add(3, 4))";
    assert_eq!(run(src, "lumelir_28d_fn").trim(), "7");
}

#[test]
fn no_shebang_program_is_unchanged() {
    // Regression: programs without shebang still compile.
    assert_eq!(run("print(1)", "lumelir_28d_noshebang").trim(), "1");
}
