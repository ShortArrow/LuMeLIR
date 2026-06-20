//! ADR 0229 — Second M7 sub-ADR. `string.find(s, sub)` widens
//! to multi-return `(start, end)` via the existing ADR 0021 /
//! 0081 multi-assign builtin dispatch (Next precedent). Single-
//! assign still truncates to the start position.

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
fn find_multireturn_start_and_end() {
    let out = run_ok(
        "local s, e = string.find(\"hello world\", \"world\")
print(s)
print(e)",
        "lumelir_strfind_mr_basic",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "11");
}

#[test]
fn find_multireturn_at_start() {
    let out = run_ok(
        "local s, e = string.find(\"abcdef\", \"abc\")
print(s)
print(e)",
        "lumelir_strfind_mr_at_start",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "3");
}

#[test]
fn find_multireturn_no_match_returns_two_nils() {
    let out = run_ok(
        "local s, e = string.find(\"hello\", \"xyz\")
print(s)
print(e)",
        "lumelir_strfind_mr_miss",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "nil");
    assert_eq!(lines[1], "nil");
}

#[test]
fn find_multireturn_single_char_pattern() {
    let out = run_ok(
        "local s, e = string.find(\"abcdef\", \"d\")
print(s)
print(e)",
        "lumelir_strfind_mr_single_char",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "4");
}
