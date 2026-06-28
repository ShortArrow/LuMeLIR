//! ADR 0280 — N4-C: pattern anchors `^` (start) and `$` (end).

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
fn caret_matches_at_start() {
    let out = run_ok("print(string.match(\"abcdef\", \"^abc\"))", "n4c_caret_yes");
    assert_eq!(out.trim(), "abc");
}

#[test]
fn caret_rejects_match_not_at_start() {
    let out = run_ok(
        "local r = string.match(\"xabcdef\", \"^abc\")
if r == nil then print(\"nil\") else print(r) end",
        "n4c_caret_no",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn dollar_matches_at_end() {
    let out = run_ok(
        "print(string.match(\"abcdef\", \"def$\"))",
        "n4c_dollar_yes",
    );
    assert_eq!(out.trim(), "def");
}

#[test]
fn dollar_rejects_match_not_at_end() {
    let out = run_ok(
        "local r = string.match(\"abcdefg\", \"def$\")
if r == nil then print(\"nil\") else print(r) end",
        "n4c_dollar_no",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn both_anchors_full_string() {
    let out = run_ok(
        "print(string.match(\"hello\", \"^hello$\"))",
        "n4c_both_yes",
    );
    assert_eq!(out.trim(), "hello");
}

#[test]
fn both_anchors_fail_when_partial() {
    let out = run_ok(
        "local r = string.match(\"hello world\", \"^hello$\")
if r == nil then print(\"nil\") else print(r) end",
        "n4c_both_no",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn caret_with_class_and_quantifier() {
    // "^%d+" → digits at start.
    let out = run_ok(
        "print(string.match(\"42abc\", \"^%d+\"))",
        "n4c_caret_class",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn dollar_with_class_and_quantifier() {
    let out = run_ok(
        "print(string.match(\"abc42\", \"%d+$\"))",
        "n4c_dollar_class",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn dollar_in_middle_is_literal() {
    // `$` not at end of pattern is a literal dollar sign.
    let out = run_ok("print(string.match(\"a$b\", \"$b\"))", "n4c_dollar_literal");
    assert_eq!(out.trim(), "$b");
}
