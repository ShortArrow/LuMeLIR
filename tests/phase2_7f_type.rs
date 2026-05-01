//! Integration test: Phase 2.7f — `type(x)` builtin (ADR 0029).

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
fn type_of_number_returns_number_word() {
    assert_eq!(run("print(type(42))", "lumelir_27f_num").trim(), "number");
}

#[test]
fn type_of_string_returns_string_word() {
    assert_eq!(
        run("print(type(\"hello\"))", "lumelir_27f_str").trim(),
        "string"
    );
}

#[test]
fn type_of_true_returns_boolean_word() {
    assert_eq!(run("print(type(true))", "lumelir_27f_t").trim(), "boolean");
}

#[test]
fn type_of_false_returns_boolean_word() {
    assert_eq!(run("print(type(false))", "lumelir_27f_f").trim(), "boolean");
}

#[test]
fn type_of_nil_returns_nil_word() {
    assert_eq!(run("print(type(nil))", "lumelir_27f_n").trim(), "nil");
}

#[test]
fn type_of_function_value_returns_function_word() {
    let src = "local f = function() return 1 end
print(type(f))";
    assert_eq!(run(src, "lumelir_27f_fn").trim(), "function");
}

#[test]
fn type_of_named_function_returns_function_word() {
    let src = "local function f() return 1 end
print(type(f))";
    assert_eq!(run(src, "lumelir_27f_named_fn").trim(), "function");
}

#[test]
fn type_result_compares_equal_to_string_literal() {
    assert_eq!(
        run("print(type(1) == \"number\")", "lumelir_27f_eq").trim(),
        "true"
    );
}

#[test]
fn type_result_concatenates() {
    assert_eq!(
        run("print(\"kind: \" .. type(true))", "lumelir_27f_concat").trim(),
        "kind: boolean"
    );
}

#[test]
fn type_in_if_predicate() {
    let src = "local x = 42
if type(x) == \"number\" then print(\"yes\") else print(\"no\") end";
    assert_eq!(run(src, "lumelir_27f_if").trim(), "yes");
}
