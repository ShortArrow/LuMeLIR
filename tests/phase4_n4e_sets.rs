//! ADR 0282 — N4-E: character sets `[abc]` / `[^abc]` / `[a-z]`.

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
fn set_literal_chars() {
    let out = run_ok("print(string.match(\"hello\", \"[hel]+\"))", "n4e_lit_set");
    assert_eq!(out.trim(), "hell");
}

#[test]
fn set_negation_excludes() {
    // [^aeiou]+ — consecutive consonants.
    let out = run_ok(
        "print(string.match(\"hello\", \"[^aeiou]+\"))",
        "n4e_neg_set",
    );
    assert_eq!(out.trim(), "h");
}

#[test]
fn set_range_lowercase() {
    let out = run_ok(
        "print(string.match(\"XYZ42abc\", \"[a-z]+\"))",
        "n4e_range_low",
    );
    assert_eq!(out.trim(), "abc");
}

#[test]
fn set_range_digit() {
    let out = run_ok(
        "print(string.match(\"abc42def\", \"[0-9]+\"))",
        "n4e_range_digit",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn set_class_inside() {
    // [%d%s] matches digits or whitespace; trim() would eat the
    // leading space we're checking, so compare against \n-suffixed.
    let out = run_ok(
        "print(string.match(\"hi 42\", \"[%d%s]+\"))",
        "n4e_class_in_set",
    );
    assert_eq!(out, " 42\n");
}

#[test]
fn set_combined_range_and_literal() {
    // [A-Za-z_] — identifier-leading char class.
    let out = run_ok(
        "print(string.match(\"42_name\", \"[A-Za-z_]+\"))",
        "n4e_id_lead",
    );
    assert_eq!(out.trim(), "_name");
}

#[test]
fn set_negation_with_class() {
    // [^%s]+ — any non-whitespace run.
    let out = run_ok(
        "print(string.match(\"   hello world\", \"[^%s]+\"))",
        "n4e_neg_class",
    );
    assert_eq!(out.trim(), "hello");
}

#[test]
fn set_quantifier_question() {
    // [aeiou]? — optional vowel.
    let out = run_ok("print(string.match(\"sky\", \"s[aeiou]?\"))", "n4e_q_vowel");
    // No vowel after 's' → match is just "s".
    assert_eq!(out.trim(), "s");
}

#[test]
fn set_inside_capture() {
    let out = run_ok(
        "print(string.match(\"=42=\", \"=([0-9]+)=\"))",
        "n4e_cap_set",
    );
    assert_eq!(out.trim(), "42");
}

#[test]
fn set_no_match_returns_nil() {
    let out = run_ok(
        "local r = string.match(\"abc\", \"[0-9]\")
if r == nil then print(\"nil\") else print(r) end",
        "n4e_nil",
    );
    assert_eq!(out.trim(), "nil");
}
