//! Phase 2.devinfra-stdout-fwrite (ADR 0117): pin the `print`
//! path's binary-safe stdout behaviour. Sibling to the io.write
//! pins in `tests/phase2_stdlib_io.rs`.
//!
//! Before ADR 0117 the codegen funneled every Lua-value string
//! through `printf("%.*s", len, data)`, which per POSIX `%s`
//! semantics stops at the first embedded NUL — defeating ADR
//! 0112's boxed-string ABI promise (Lua §2.4 "8-bit clean").
//! ADR 0117 swapped the chokepoint to `fwrite(data, 1, len,
//! stdout)` and these tests pin the post-fix behaviour for
//! `print` (which appends a trailing newline).

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    result
}

fn run_ok(src: &str, output_name: &str) -> Vec<u8> {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    out.stdout
}

#[test]
fn print_embedded_nul_preserves_bytes_with_newline() {
    // print outputs the string then a newline. Pre-0117 the
    // leading NUL caused %s truncation → empty + newline. Post-
    // 0117 fwrite emits all 4 bytes followed by '\n' (also via
    // emit_print_string_obj of the s_newline boxed object).
    let stdout = run_ok(
        "print(string.char(0, 65, 0, 66))",
        "lumelir_print_embedded_nul_preserves",
    );
    assert_eq!(stdout, &[0u8, 65, 0, 66, b'\n'][..]);
}

#[test]
fn print_middle_nul_preserves_bytes_with_newline() {
    let stdout = run_ok(
        "print(string.char(65, 0, 66))",
        "lumelir_print_middle_nul_preserves",
    );
    assert_eq!(stdout, &[65u8, 0, 66, b'\n'][..]);
}

#[test]
fn print_ascii_regression_with_newline() {
    // NUL-free regression pin: behaviour unchanged through the
    // fwrite swap.
    let stdout = run_ok("print(\"ABC\")", "lumelir_print_ascii_regression");
    assert_eq!(stdout, &[65u8, 66, 67, b'\n'][..]);
}
