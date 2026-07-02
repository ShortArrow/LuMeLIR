//! ADR 0290 — N7-19: utf8.offset(s, n) 2-arg form.

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
fn offset_first_codepoint_is_1() {
    let out = run_ok("print(utf8.offset(\"hello\", 1))", "n7_19_1");
    assert_eq!(out.trim(), "1");
}

#[test]
fn offset_second_ascii_is_2() {
    let out = run_ok("print(utf8.offset(\"hello\", 2))", "n7_19_2");
    assert_eq!(out.trim(), "2");
}

#[test]
fn offset_after_2byte_codepoint() {
    // "é" = 2 bytes; the codepoint after it starts at byte 3.
    let out = run_ok(
        "print(utf8.offset(utf8.char(0xE9) .. \"x\", 2))",
        "n7_19_after_2b",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn offset_end_boundary() {
    // "abc" has 3 codepoints; offset 4 is the position past the last
    // (which is byte 4 = len + 1).
    let out = run_ok("print(utf8.offset(\"abc\", 4))", "n7_19_end");
    assert_eq!(out.trim(), "4");
}

#[test]
fn offset_zero_returns_1() {
    let out = run_ok("print(utf8.offset(\"hello\", 0))", "n7_19_zero");
    assert_eq!(out.trim(), "1");
}
