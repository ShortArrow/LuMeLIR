//! Integration test: Phase 2.7k — extended string escapes (ADR 0039).
//! `\a \b \f \v` (single-char), `\xHH` (hex byte, 0..=0x7F),
//! `\ddd` (decimal byte, 0..=127, 1-3 digits).

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
fn hex_escape_emits_ascii_byte() {
    // `\x41` = 'A'; `print` adds the trailing newline.
    assert_eq!(
        run("print(\"\\x41\\x42\\x43\")", "lumelir_27k_hex"),
        "ABC\n"
    );
}

#[test]
fn decimal_escape_emits_ascii_byte() {
    // 65, 66, 67 = "ABC".
    assert_eq!(run("print(\"\\65\\66\\67\")", "lumelir_27k_dec"), "ABC\n");
}

#[test]
fn decimal_escape_three_digit_padded() {
    assert_eq!(run("print(\"\\065\")", "lumelir_27k_dec3"), "A\n");
}

#[test]
fn alert_escape_emits_bell_byte() {
    // `\a` = 0x07. We can't see it on stdout but `#s` reports 1.
    assert_eq!(run("print(#\"\\a\")", "lumelir_27k_alert").trim(), "1");
}

#[test]
fn form_feed_and_vtab_escapes_lengths() {
    // `\f` (0x0C) + `\v` (0x0B) = 2 bytes.
    assert_eq!(run("print(#\"\\f\\v\")", "lumelir_27k_fv").trim(), "2");
}

#[test]
fn hex_escape_in_concat() {
    // `\x21` = '!'.
    assert_eq!(
        run("print(\"hi\" .. \"\\x21\")", "lumelir_27k_concat").trim(),
        "hi!"
    );
}

#[test]
fn hex_escape_out_of_ascii_range_is_lex_error() {
    assert!(lumelir::parser::parse("print(\"\\xff\")").is_err());
}

#[test]
fn decimal_escape_out_of_ascii_range_is_lex_error() {
    assert!(lumelir::parser::parse("print(\"\\200\")").is_err());
}
