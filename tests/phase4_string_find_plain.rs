//! ADR 0228 — M7-A first sub-ADR: `string.find(s, sub)` literal
//! substring search. Single-return TaggedValue (Number-or-Nil).
//! Magic-char pattern semantics + multi-return (start, end)
//! deferred to subsequent M7 sub-ADRs.

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
fn find_substring_returns_1_indexed_start() {
    assert_eq!(
        run_ok(
            "print(string.find(\"hello world\", \"world\"))",
            "lumelir_strfind_basic"
        )
        .trim(),
        "7"
    );
}

#[test]
fn find_missing_substring_returns_nil() {
    assert_eq!(
        run_ok(
            "print(string.find(\"hello\", \"xyz\"))",
            "lumelir_strfind_miss"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn find_empty_pattern_returns_one() {
    assert_eq!(
        run_ok("print(string.find(\"abc\", \"\"))", "lumelir_strfind_empty").trim(),
        "1"
    );
}

#[test]
fn find_pattern_at_start_returns_one() {
    assert_eq!(
        run_ok(
            "print(string.find(\"hello\", \"hel\"))",
            "lumelir_strfind_start"
        )
        .trim(),
        "1"
    );
}

#[test]
fn find_composes_with_if() {
    let out = run_ok(
        "local p = string.find(\"prefix-rest\", \"prefix\")
if p then print(\"found\") else print(\"missed\") end",
        "lumelir_strfind_in_if",
    );
    assert_eq!(out.trim(), "found");
}

#[test]
fn find_missing_composes_with_if_else() {
    let out = run_ok(
        "local p = string.find(\"hello\", \"world\")
if p then print(\"found\") else print(\"missed\") end",
        "lumelir_strfind_in_if_else",
    );
    assert_eq!(out.trim(), "missed");
}
