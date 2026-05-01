//! Integration test: Phase 2.7l — `\u{XXXX}` codepoint escapes
//! and `\z` whitespace-skip escape (ADR 0040).

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn unicode_escape_ascii_emits_one_byte() {
    assert_eq!(
        run("print(\"\\u{48}\\u{69}\")", "lumelir_27l_uascii"),
        "Hi\n"
    );
}

#[test]
fn unicode_escape_two_byte_sequence() {
    // 'é' = U+00E9, UTF-8 0xC3 0xA9.
    let stdout = run("print(\"caf\\u{e9}\")", "lumelir_27l_u2byte");
    assert_eq!(stdout, "café\n");
}

#[test]
fn unicode_escape_three_byte_sequence() {
    // 'あ' = U+3042, UTF-8 0xE3 0x81 0x82.
    let stdout = run("print(\"\\u{3042}\\u{3044}\")", "lumelir_27l_u3byte");
    assert_eq!(stdout, "あい\n");
}

#[test]
fn unicode_escape_four_byte_emoji() {
    // 😀 = U+1F600, UTF-8 0xF0 0x9F 0x98 0x80.
    let stdout = run("print(\"\\u{1F600}\")", "lumelir_27l_uemoji");
    assert_eq!(stdout, "😀\n");
}

#[test]
fn unicode_escape_length_via_hash_counts_bytes() {
    // `#s` reports byte count — Lua semantics. 'é' is 2 bytes.
    assert_eq!(run("print(#\"\\u{e9}\")", "lumelir_27l_ulen").trim(), "2");
}

#[test]
fn z_escape_collapses_inline_whitespace_run() {
    // `\z` skips the whitespace including the newline; no LF in
    // the output between "long " and "string".
    let src = "print(\"long \\z\n            string\")";
    assert_eq!(run(src, "lumelir_27l_z_inline"), "long string\n");
}

#[test]
fn z_escape_then_no_whitespace_is_noop() {
    assert_eq!(run("print(\"a\\zb\")", "lumelir_27l_z_noop"), "ab\n");
}

#[test]
fn unicode_escape_invalid_codepoint_is_lex_error() {
    // U+D800 is a UTF-16 surrogate, not a valid Unicode scalar.
    assert!(lumelir::parser::parse("print(\"\\u{D800}\")").is_err());
}
