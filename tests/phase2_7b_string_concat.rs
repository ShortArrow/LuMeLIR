//! Integration test: Phase 2.7b — string concat `..` and runtime
//! string equality (ADR 0025).

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
fn concat_two_literals() {
    assert_eq!(
        run("print(\"foo\" .. \"bar\")", "lumelir_27b_lit2").trim(),
        "foobar"
    );
}

#[test]
fn concat_three_literals_right_associative() {
    // `a..b..c` parses as `a..(b..c)` — right associative.
    assert_eq!(
        run("print(\"a\" .. \"b\" .. \"c\")", "lumelir_27b_lit3").trim(),
        "abc"
    );
}

#[test]
fn concat_locals() {
    let src = "local a = \"foo\"
local b = \"bar\"
print(a..b)";
    assert_eq!(run(src, "lumelir_27b_locals").trim(), "foobar");
}

#[test]
fn concat_with_space_literal() {
    assert_eq!(
        run(
            "print(\"hello\" .. \" \" .. \"world\")",
            "lumelir_27b_hello"
        )
        .trim(),
        "hello world"
    );
}

#[test]
fn string_equal_returns_true() {
    assert_eq!(
        run("print(\"abc\" == \"abc\")", "lumelir_27b_eq_t").trim(),
        "true"
    );
}

#[test]
fn string_equal_different_returns_false() {
    assert_eq!(
        run("print(\"abc\" == \"xyz\")", "lumelir_27b_eq_f").trim(),
        "false"
    );
}

#[test]
fn string_not_equal_returns_true_when_different() {
    assert_eq!(
        run("print(\"abc\" ~= \"xyz\")", "lumelir_27b_ne_t").trim(),
        "true"
    );
}

#[test]
fn string_not_equal_returns_false_when_same() {
    assert_eq!(
        run("print(\"abc\" ~= \"abc\")", "lumelir_27b_ne_f").trim(),
        "false"
    );
}

#[test]
fn concat_then_length() {
    let src = "print(#(\"foo\" .. \"barbaz\"))";
    assert_eq!(run(src, "lumelir_27b_concat_len").trim(), "9");
}

#[test]
fn string_eq_used_in_if_condition() {
    let src = "local s = \"hello\"
if s == \"hello\" then print(1) else print(0) end";
    assert_eq!(run(src, "lumelir_27b_eq_if").trim(), "1");
}

#[test]
fn concat_number_to_string_is_static_error() {
    // `..` requires both operands to be String. Number → reject.
    let chunk = lumelir::parser::parse("print(\"x\" .. 1)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn concat_in_user_function() {
    let src = "local function greet(name) return \"hi \" .. name end
print(greet(\"lua\"))";
    assert_eq!(run(src, "lumelir_27b_greet").trim(), "hi lua");
}
