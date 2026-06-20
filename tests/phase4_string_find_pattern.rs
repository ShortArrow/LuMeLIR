//! ADR 0231 — M7-D: pattern matcher with char classes + `.`
//! wildcard. The runtime helper `lumelir_string_match_extents`
//! handles literal bytes, `%d` `%a` `%s` `%w` `%p` `%l` `%u`
//! `%x` `%c` and their complements, `.` (any char), `%X` magic
//! escapes. No quantifiers / sets / anchors / captures yet.

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
fn find_digit_class_position() {
    // %d at position 4 in "abc1def".
    assert_eq!(
        run_ok(
            "print(string.find(\"abc1def\", \"%d\"))",
            "lumelir_strfind_digit"
        )
        .trim(),
        "4"
    );
}

#[test]
fn find_alpha_class_at_start() {
    assert_eq!(
        run_ok(
            "print(string.find(\"abc123\", \"%a\"))",
            "lumelir_strfind_alpha"
        )
        .trim(),
        "1"
    );
}

#[test]
fn find_wildcard_dot_matches_any_char() {
    // "a.b" pattern matches "axb" since `.` is wildcard.
    assert_eq!(
        run_ok(
            "print(string.find(\"foo axb bar\", \"a.b\"))",
            "lumelir_strfind_dot"
        )
        .trim(),
        "5"
    );
}

#[test]
fn find_escaped_dot_matches_literal_dot() {
    assert_eq!(
        run_ok(
            "print(string.find(\"file.lua\", \"%.lua\"))",
            "lumelir_strfind_esc_dot"
        )
        .trim(),
        "5"
    );
}

#[test]
fn find_multi_class_pattern_returns_extents() {
    // `%a%d` = letter then digit. Match at "b1" in "ab1c".
    let out = run_ok(
        "local s, e = string.find(\"ab1c\", \"%a%d\")
print(s)
print(e)",
        "lumelir_strfind_multi_class",
    );
    let lines: Vec<&str> = out.lines().collect();
    // "ab1c" — match "a%a" then "b%d"? Trying "a" + "b" no (%d miss).
    // Trying "b" + "1": %a=b ok, %d=1 ok. start=2, end=3.
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "3");
}

#[test]
fn find_pattern_miss_returns_nil() {
    assert_eq!(
        run_ok(
            "print(string.find(\"abcdef\", \"%d\"))",
            "lumelir_strfind_no_digit"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn find_complement_class_finds_non_digit() {
    // %D in "123x4" matches at position 4 ('x').
    assert_eq!(
        run_ok(
            "print(string.find(\"123x4\", \"%D\"))",
            "lumelir_strfind_compl"
        )
        .trim(),
        "4"
    );
}
