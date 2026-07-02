//! ADR 0288 — N7-17: utf8.char(n) single-codepoint form.

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
fn utf8_char_ascii() {
    let out = run_ok("print(utf8.char(65))", "n7_17_ascii");
    assert_eq!(out.trim(), "A");
}

#[test]
fn utf8_char_ascii_zero_len_via_len() {
    // utf8.char(65) → "A"; utf8.len should report 1 codepoint.
    let out = run_ok("print(utf8.len(utf8.char(65)))", "n7_17_ascii_len");
    assert_eq!(out.trim(), "1");
}

#[test]
fn utf8_char_two_byte() {
    // é = U+00E9, 2 UTF-8 bytes 0xC3 0xA9. Verify via utf8.len.
    let out = run_ok("print(utf8.len(utf8.char(0xE9)))", "n7_17_2byte");
    assert_eq!(out.trim(), "1");
}

#[test]
fn utf8_char_three_byte() {
    // U+3042 (ひらがな "あ") = 3 UTF-8 bytes.
    let out = run_ok("print(utf8.len(utf8.char(0x3042)))", "n7_17_3byte");
    assert_eq!(out.trim(), "1");
}

#[test]
fn utf8_char_four_byte() {
    // U+1F600 (😀) = 4 UTF-8 bytes.
    let out = run_ok("print(utf8.len(utf8.char(0x1F600)))", "n7_17_4byte");
    assert_eq!(out.trim(), "1");
}
