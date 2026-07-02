//! ADR 0289 — N7-18: utf8.codepoint(s, i) single-position form.

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
fn codepoint_ascii_first() {
    let out = run_ok("print(utf8.codepoint(\"ABC\", 1))", "n7_18_ascii");
    assert_eq!(out.trim(), "65");
}

#[test]
fn codepoint_ascii_second() {
    let out = run_ok("print(utf8.codepoint(\"ABC\", 2))", "n7_18_ascii2");
    assert_eq!(out.trim(), "66");
}

#[test]
fn codepoint_roundtrip_char() {
    let out = run_ok(
        "print(utf8.codepoint(utf8.char(0x3042), 1))",
        "n7_18_roundtrip",
    );
    assert_eq!(out.trim(), "12354"); // 0x3042
}

#[test]
fn codepoint_two_byte() {
    // U+00E9 (é). Ensure decode works on 2-byte input.
    let out = run_ok("print(utf8.codepoint(utf8.char(0xE9), 1))", "n7_18_2byte");
    assert_eq!(out.trim(), "233");
}

#[test]
fn codepoint_four_byte() {
    let out = run_ok(
        "print(utf8.codepoint(utf8.char(0x1F600), 1))",
        "n7_18_4byte",
    );
    assert_eq!(out.trim(), "128512"); // 0x1F600
}
