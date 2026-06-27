//! ADR 0278 — N4-A: string.match wires up the pattern engine
//! (literal char, `.` wildcard, `%a %d %s %w %p` classes).
//! Pattern engine itself shipped under ADR 0231; this ADR makes
//! string.match return the matched substring (not the pattern).

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
fn match_digit_class_returns_actual_digit() {
    // %d matches a digit; the result is the matched char "7",
    // not the pattern "%d".
    let out = run_ok("print(string.match(\"abc7def\", \"%d\"))", "n4a_digit");
    assert_eq!(out.trim(), "7");
}

#[test]
fn match_alpha_class_first_letter() {
    let out = run_ok("print(string.match(\"   hello\", \"%a\"))", "n4a_alpha");
    assert_eq!(out.trim(), "h");
}

#[test]
fn match_wildcard_dot_matches_any_first_char() {
    let out = run_ok("print(string.match(\"xyz\", \".\"))", "n4a_dot");
    assert_eq!(out.trim(), "x");
}

#[test]
fn match_whitespace_class() {
    // print emits the space + newline; .trim() would eat the space.
    let out = run_ok("print(string.match(\"a b\", \"%s\"))", "n4a_space");
    assert_eq!(out, " \n");
}

#[test]
fn match_word_class_against_punctuation_fails() {
    // %w is alphanumeric; "..." has none → nil.
    let out = run_ok(
        "local r = string.match(\"...\", \"%w\")
if r == nil then print(\"nil\") else print(r) end",
        "n4a_word_nil",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn match_punctuation_class() {
    let out = run_ok("print(string.match(\"abc!\", \"%p\"))", "n4a_punct");
    assert_eq!(out.trim(), "!");
}

#[test]
fn match_multi_char_pattern_returns_full_match() {
    // Pattern "%d%d" should match the two consecutive digits.
    let out = run_ok("print(string.match(\"foo42bar\", \"%d%d\"))", "n4a_two_d");
    assert_eq!(out.trim(), "42");
}

#[test]
fn match_literal_in_pattern_returns_literal() {
    let out = run_ok("print(string.match(\"abc\", \"b\"))", "n4a_lit");
    assert_eq!(out.trim(), "b");
}

#[test]
fn match_mixed_class_and_literal() {
    // "%d:" matches digit followed by colon.
    let out = run_ok(
        "print(string.match(\"time=9:30\", \"%d:\"))",
        "n4a_mixed",
    );
    assert_eq!(out.trim(), "9:");
}
