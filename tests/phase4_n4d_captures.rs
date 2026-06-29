//! ADR 0281 — N4-D: single capture `(...)` returned by string.match.

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
fn capture_returns_only_inner() {
    // Pattern "a(%d+)b" captures the digits between `a` and `b`.
    let out = run_ok(
        "print(string.match(\"a42b\", \"a(%d+)b\"))",
        "n4d_inner_digits",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn capture_empty_when_inside_star() {
    // Capture of zero matches → empty string is still a captured value.
    let out = run_ok(
        "local r = string.match(\"ab\", \"a(%d*)b\")
if r == \"\" then print(\"empty\") else print(r) end",
        "n4d_empty_cap",
    );
    assert_eq!(out.trim(), "empty");
}

#[test]
fn capture_at_start_of_pattern() {
    let out = run_ok(
        "print(string.match(\"abc123\", \"(%a+)\"))",
        "n4d_start_cap",
    );
    assert_eq!(out.trim(), "abc");
}

#[test]
fn capture_with_anchor() {
    let out = run_ok(
        "print(string.match(\"hello world\", \"^(%a+)\"))",
        "n4d_anchored_cap",
    );
    assert_eq!(out.trim(), "hello");
}

#[test]
fn no_capture_returns_full_match() {
    // Without parens, string.match returns the entire match.
    let out = run_ok("print(string.match(\"foo42bar\", \"%d+\"))", "n4d_no_cap");
    assert_eq!(out.trim(), "42");
}

#[test]
fn capture_with_dollar_anchor() {
    let out = run_ok(
        "print(string.match(\"file.txt\", \"%.(%a+)$\"))",
        "n4d_dollar_cap",
    );
    assert_eq!(out.trim(), "txt");
}

#[test]
fn capture_no_match_returns_nil() {
    let out = run_ok(
        "local r = string.match(\"abc\", \"(%d+)\")
if r == nil then print(\"nil\") else print(r) end",
        "n4d_cap_nil",
    );
    assert_eq!(out.trim(), "nil");
}
