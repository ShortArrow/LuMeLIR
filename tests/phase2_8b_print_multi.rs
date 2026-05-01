//! Integration test: Phase 2.8b — variadic `print(a, b, ...)`
//! (ADR 0032).

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
fn print_zero_args_emits_bare_newline() {
    assert_eq!(run("print()", "lumelir_28b_zero"), "\n");
}

#[test]
fn print_single_arg_unchanged_after_2_8b() {
    // Regression: existing single-arg path still works.
    assert_eq!(run("print(42)", "lumelir_28b_one").trim(), "42");
}

#[test]
fn print_two_strings_separated_by_tab() {
    assert_eq!(run("print(\"a\", \"b\")", "lumelir_28b_two").trim(), "a\tb");
}

#[test]
fn print_three_numbers_with_tabs() {
    assert_eq!(run("print(1, 2, 3)", "lumelir_28b_three").trim(), "1\t2\t3");
}

#[test]
fn print_mixed_kinds() {
    assert_eq!(
        run("print(\"x\", 42, true, nil)", "lumelir_28b_mixed").trim(),
        "x\t42\ttrue\tnil"
    );
}

#[test]
fn print_string_then_tostring() {
    assert_eq!(
        run("print(\"count:\", tostring(7))", "lumelir_28b_count").trim(),
        "count:\t7"
    );
}

#[test]
fn print_bool_pair() {
    assert_eq!(
        run("print(true, false)", "lumelir_28b_bools").trim(),
        "true\tfalse"
    );
}

#[test]
fn print_in_user_function() {
    let src = "local function dump(a, b) print(a, b) end
dump(\"first\", \"second\")";
    assert_eq!(run(src, "lumelir_28b_fn").trim(), "first\tsecond");
}

#[test]
fn print_call_emits_trailing_newline() {
    // The full output, including the trailing newline.
    assert_eq!(
        run("print(\"hi\", \"there\")", "lumelir_28b_nl"),
        "hi\tthere\n"
    );
}
