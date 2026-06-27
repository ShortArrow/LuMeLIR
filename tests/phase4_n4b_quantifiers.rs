//! ADR 0279 — N4-B: pattern quantifiers `*` `+` `?` (greedy).

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
fn star_greedy_zero_or_more_digits() {
    // "abc42def" with "%d*" greedily matches "" at position 1
    // (before 'a'). Greedy * matches 0+ → first attempt is empty.
    let out = run_ok(
        "local r = string.match(\"abc42def\", \"%d*\")
if r == \"\" then print(\"empty\") else print(r) end",
        "n4b_star_empty",
    );
    assert_eq!(out.trim(), "empty");
}

#[test]
fn plus_greedy_one_or_more_digits() {
    // Pattern "%d+" requires at least one digit.
    let out = run_ok("print(string.match(\"abc42def\", \"%d+\"))", "n4b_plus");
    assert_eq!(out.trim(), "42");
}

#[test]
fn plus_no_match_returns_nil() {
    let out = run_ok(
        "local r = string.match(\"abc\", \"%d+\")
if r == nil then print(\"nil\") else print(r) end",
        "n4b_plus_nil",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn question_zero_or_one() {
    // "%d?" matches "" at position 1 (greedy tries one, but no
    // digit at start, falls back to empty match).
    let out = run_ok(
        "local r = string.match(\"abc\", \"%d?\")
if r == \"\" then print(\"empty\") else print(r) end",
        "n4b_q_empty",
    );
    assert_eq!(out.trim(), "empty");
}

#[test]
fn question_followed_by_required_atom() {
    // "%d?:" → optional digit followed by colon.
    let out = run_ok("print(string.match(\"x:7\", \"%d?:\"))", "n4b_q_then_colon");
    assert_eq!(out.trim(), ":");
}

#[test]
fn star_greedy_backtracks_for_trailing_atom() {
    // ".*z" should consume up to the last z.
    let out = run_ok(
        "print(string.match(\"abczdefz\", \".*z\"))",
        "n4b_star_back",
    );
    assert_eq!(out.trim(), "abczdefz");
}

#[test]
fn plus_backtracks_to_satisfy_trailing() {
    // "%d+5" → digits + literal 5. "1235" → "1235", "12345" → "12345".
    let out = run_ok(
        "print(string.match(\"abc12345xyz\", \"%d+5\"))",
        "n4b_plus_back",
    );
    assert_eq!(out.trim(), "12345");
}

#[test]
fn star_after_dot_matches_until_end() {
    let out = run_ok("print(string.match(\"hello\", \".*\"))", "n4b_dot_star");
    assert_eq!(out.trim(), "hello");
}

#[test]
fn question_with_class_complement() {
    // "%a+%d?" — letters then optional digit.
    let out = run_ok("print(string.match(\"abc7\", \"%a+%d?\"))", "n4b_alpha_q");
    assert_eq!(out.trim(), "abc7");
}
