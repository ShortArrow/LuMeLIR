//! Integration test: Phase 2.8a — Lua single-line `--` comments
//! (ADR 0031).

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
fn comment_at_top_of_file() {
    let src = "-- this is a top-of-file comment
print(1)";
    assert_eq!(run(src, "lumelir_28a_top").trim(), "1");
}

#[test]
fn inline_comment_after_call() {
    let src = "print(42) -- inline";
    assert_eq!(run(src, "lumelir_28a_inline").trim(), "42");
}

#[test]
fn comment_only_file_compiles() {
    // A file with only a comment lowers to an empty chunk and
    // produces a runnable binary that prints nothing.
    let src = "-- nothing to do here";
    assert_eq!(run(src, "lumelir_28a_only").trim(), "");
}

#[test]
fn comment_between_statements() {
    let src = "print(1)
-- separator
print(2)";
    assert_eq!(run(src, "lumelir_28a_between").trim(), "1\n2");
}

#[test]
fn comment_in_middle_of_expression() {
    let src = "print(1 -- partial sum
    + 2)";
    assert_eq!(run(src, "lumelir_28a_mid_expr").trim(), "3");
}

#[test]
fn comment_inside_function_body() {
    let src = "local function f(x)
  -- doubles its argument
  return x * 2
end
print(f(7))";
    assert_eq!(run(src, "lumelir_28a_in_fn").trim(), "14");
}

#[test]
fn comment_does_not_consume_following_newline() {
    // The `\n` after a comment must still terminate it, allowing
    // the next line to lex normally even when the comment occupies
    // the whole preceding line.
    let src = "-- header\nprint(\"after\")";
    assert_eq!(run(src, "lumelir_28a_nl").trim(), "after");
}
