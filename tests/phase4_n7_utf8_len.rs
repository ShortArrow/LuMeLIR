//! ADR 0277 — N7-16: utf8.len(s) counts codepoints.

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
fn utf8_len_ascii_matches_byte_len() {
    let out = run_ok("print(utf8.len(\"hello\"))", "n7_utf8_ascii");
    assert_eq!(out.trim(), "5");
}

#[test]
fn utf8_len_multibyte_counts_codepoints() {
    // "héllo" — é is 2 bytes (0xC3 0xA9), the other 4 are ASCII.
    // Byte length is 6 but codepoint count is 5.
    let out = run_ok("print(utf8.len(\"héllo\"))", "n7_utf8_multi");
    assert_eq!(out.trim(), "5");
}

#[test]
fn utf8_len_empty() {
    let out = run_ok("print(utf8.len(\"\"))", "n7_utf8_empty");
    assert_eq!(out.trim(), "0");
}
