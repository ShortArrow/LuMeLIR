//! ADR 0287 — N4-H: pattern `%bxy` balanced match + `%f[set]` frontier.

use std::process::Command;

fn run_ok(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let r = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    assert!(r.status.success(), "binary should exit 0: {r:?}");
    String::from_utf8_lossy(&r.stdout).into_owned()
}

#[test]
fn balanced_parens_simple() {
    let out = run_ok(
        "print(string.match(\"x(abc)y\", \"%b()\"))",
        "n4h_paren_simple",
    );
    assert_eq!(out.trim(), "(abc)");
}

#[test]
fn balanced_parens_nested() {
    let out = run_ok(
        "print(string.match(\"pre(a(b)c)post\", \"%b()\"))",
        "n4h_paren_nested",
    );
    assert_eq!(out.trim(), "(a(b)c)");
}

#[test]
fn balanced_braces() {
    let out = run_ok("print(string.match(\"x{a{b}}y\", \"%b{}\"))", "n4h_braces");
    assert_eq!(out.trim(), "{a{b}}");
}

#[test]
fn balanced_unbalanced_fails() {
    let out = run_ok(
        "local r = string.match(\"x(abc\", \"%b()\")
if r == nil then print(\"nil\") else print(r) end",
        "n4h_unbalanced",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn frontier_word_boundary() {
    // %f[%w] finds a word boundary — beginning of the first word.
    // Leading spaces would be trim()'d away; use \n-suffixed compare.
    let out = run_ok(
        "print(string.gsub(\"  hello world\", \"%f[%w]\", \"|\"))",
        "n4h_frontier_word",
    );
    assert_eq!(out, "  |hello |world\n");
}

#[test]
fn frontier_end_of_word() {
    // %f[%W] matches at the transition from word to non-word.
    // "hello world" → after 'o' (before ' ') and after 'd' (end).
    let out = run_ok(
        "print(string.gsub(\"hello world\", \"%f[%W]\", \"|\"))",
        "n4h_frontier_endword",
    );
    // Boundaries: after "hello" and after "world" (end of string).
    assert_eq!(out.trim(), "hello| world|");
}

#[test]
fn frontier_start_of_string() {
    // %f[%a] at position 0 with 'a' as first char — prev = \0 (not
    // in %a), cur = 'a' (in %a) → matches at pos 0.
    let out = run_ok(
        "print(string.gsub(\"abc\", \"%f[%a]\", \"X\"))",
        "n4h_frontier_start",
    );
    assert_eq!(out.trim(), "Xabc");
}
