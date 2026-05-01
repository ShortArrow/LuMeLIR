//! Integration test: Phase 2.7a — string literals, `print(string)`,
//! and the `#` length operator (ADR 0024).

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
fn print_string_literal_double_quoted() {
    assert_eq!(run("print(\"hello\")", "lumelir_27a_dq").trim(), "hello");
}

#[test]
fn print_string_literal_single_quoted() {
    assert_eq!(run("print('world')", "lumelir_27a_sq").trim(), "world");
}

#[test]
fn print_string_via_local() {
    let src = "local s = \"lua\"\nprint(s)";
    assert_eq!(run(src, "lumelir_27a_local").trim(), "lua");
}

#[test]
fn length_of_string_literal() {
    assert_eq!(run("print(#\"hello\")", "lumelir_27a_len_lit").trim(), "5");
}

#[test]
fn length_of_string_local() {
    let src = "local s = \"abcdef\"\nprint(#s)";
    assert_eq!(run(src, "lumelir_27a_len_local").trim(), "6");
}

#[test]
fn length_of_empty_string() {
    assert_eq!(run("print(#\"\")", "lumelir_27a_len_empty").trim(), "0");
}

#[test]
fn basic_escape_sequences() {
    let stdout = run("print(\"a\\tb\")", "lumelir_27a_esc");
    assert_eq!(stdout, "a\tb\n");
}

#[test]
fn duplicate_literals_share_one_global() {
    // A program with two occurrences of the same literal should
    // still produce the correct output without emitting the global
    // twice. The dedup is internal — observable result is unchanged.
    let src = "print(\"x\")\nprint(\"x\")";
    assert_eq!(run(src, "lumelir_27a_dup").trim(), "x\nx");
}

#[test]
fn length_of_non_string_is_static_error() {
    let chunk = lumelir::parser::parse("print(#42)").unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn unterminated_string_is_lex_error() {
    assert!(lumelir::parser::parse("print(\"oops").is_err());
}

#[test]
fn invalid_escape_is_lex_error() {
    // `\q` is unrecognised. (`\z` was the original sample but
    // became valid in Phase 2.7l (ADR 0040).)
    assert!(lumelir::parser::parse("print(\"a\\qb\")").is_err());
}

#[test]
fn print_int_unchanged_after_2_7a() {
    // Regression: numbers still print as before.
    assert_eq!(run("print(42)", "lumelir_27a_regress").trim(), "42");
}
