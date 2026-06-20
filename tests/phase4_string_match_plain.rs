//! ADR 0230 — M7-C: `string.match(s, sub)` literal substring
//! match. Returns the matched substring (which equals `sub` in
//! plain mode) on a hit, Nil on miss. Magic-char patterns +
//! captures deferred to the pattern matcher port.

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
fn match_substring_returns_sub() {
    assert_eq!(
        run_ok(
            "print(string.match(\"hello world\", \"world\"))",
            "lumelir_strmatch_basic"
        )
        .trim(),
        "world"
    );
}

#[test]
fn match_missing_returns_nil() {
    assert_eq!(
        run_ok(
            "print(string.match(\"hello\", \"xyz\"))",
            "lumelir_strmatch_miss"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn match_composes_with_if_then() {
    let out = run_ok(
        "local m = string.match(\"file.lua\", \".lua\")
if m then print(\"matched\") else print(\"missed\") end",
        "lumelir_strmatch_in_if",
    );
    assert_eq!(out.trim(), "matched");
}

#[test]
fn match_missing_composes_with_else_branch() {
    let out = run_ok(
        "local m = string.match(\"file.txt\", \".lua\")
if m then print(\"matched\") else print(\"missed\") end",
        "lumelir_strmatch_in_else",
    );
    assert_eq!(out.trim(), "missed");
}
