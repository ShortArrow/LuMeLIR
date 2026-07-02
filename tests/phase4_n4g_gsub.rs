//! ADR 0286 — N4-G: string.gsub(s, pat, repl) string-repl form.
//! Function-repl and multi-return (result, count) deferred.

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
fn gsub_literal_replace() {
    let out = run_ok(
        "print(string.gsub(\"hello world\", \"world\", \"lua\"))",
        "n4g_lit",
    );
    assert_eq!(out.trim(), "hello lua");
}

#[test]
fn gsub_digit_class_all() {
    let out = run_ok(
        "print(string.gsub(\"a1 b2 c3\", \"%d\", \"?\"))",
        "n4g_class",
    );
    assert_eq!(out.trim(), "a? b? c?");
}

#[test]
fn gsub_no_match_returns_original() {
    let out = run_ok("print(string.gsub(\"abc\", \"%d+\", \"X\"))", "n4g_none");
    assert_eq!(out.trim(), "abc");
}

#[test]
fn gsub_quantifier_plus_multi() {
    let out = run_ok(
        "print(string.gsub(\"1 22 333 xyz\", \"%d+\", \"N\"))",
        "n4g_plus",
    );
    assert_eq!(out.trim(), "N N N xyz");
}

#[test]
fn gsub_set_pattern() {
    // Replace vowels with '_'.
    let out = run_ok(
        "print(string.gsub(\"hello\", \"[aeiou]\", \"_\"))",
        "n4g_set",
    );
    assert_eq!(out.trim(), "h_ll_");
}

#[test]
fn gsub_anchor_only_first() {
    // ^abc only matches at the start.
    let out = run_ok(
        "print(string.gsub(\"abcabc\", \"^abc\", \"X\"))",
        "n4g_anchor",
    );
    assert_eq!(out.trim(), "Xabc");
}

#[test]
fn gsub_empty_replacement_deletes() {
    let out = run_ok("print(string.gsub(\"a1b2c3\", \"%d\", \"\"))", "n4g_delete");
    assert_eq!(out.trim(), "abc");
}

#[test]
fn gsub_replacement_longer_than_match() {
    let out = run_ok("print(string.gsub(\"1+2\", \"%d\", \"NUM\"))", "n4g_longer");
    assert_eq!(out.trim(), "NUM+NUM");
}
