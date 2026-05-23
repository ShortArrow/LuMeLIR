//! Phase 2.7x-stdlib-io-write (ADR 0116): `io.write(...)`.
//!
//! Variadic stdout writer. Sibling of `print` without the tab
//! separator and without a trailing newline. Per Lua §6.6, only
//! Number and String args are valid; Bool/Nil/Function/Table
//! reject at HIR time.
//!
//! ABI regression pins: embedded NUL strings (ADR 0112 boxed
//! string ABI) must roundtrip through `io.write` unchanged.

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

// --- Happy paths ---

#[test]
fn io_write_basic_string() {
    // io.write("hello") → stdout "hello" (no newline).
    let stdout = run_ok("io.write(\"hello\")", "lumelir_io_write_basic");
    assert_eq!(stdout, b"hello");
}

#[test]
fn io_write_multiple_strings_concatenate() {
    // No separator between args.
    let stdout = run_ok("io.write(\"a\", \"b\", \"c\")", "lumelir_io_write_multi");
    assert_eq!(stdout, b"abc");
}

#[test]
fn io_write_empty_arity_zero() {
    // io.write() with 0 args writes nothing but exits success.
    let stdout = run_ok("io.write()", "lumelir_io_write_empty");
    assert_eq!(stdout, b"");
}

#[test]
fn io_write_number_coerces_to_string() {
    // Numbers are coerced via the same printf path as `print(n)`.
    let stdout = run_ok("io.write(42)", "lumelir_io_write_number");
    assert_eq!(stdout, b"42");
}

#[test]
fn io_write_mixed_number_and_string() {
    let stdout = run_ok("io.write(\"x=\", 42)", "lumelir_io_write_mixed");
    assert_eq!(stdout, b"x=42");
}

#[test]
fn io_write_embedded_nul_preserves_bytes() {
    // ADR 0117 resolved the ADR 0112 carry-over: `emit_print_string_obj`
    // now uses `fwrite(data, 1, len, stdout)` instead of
    // `printf("%.*s", ...)`, so binary-safe stdout writes preserve
    // every byte including embedded NULs. Lua §2.4: "Lua is also
    // 8-bit clean: strings can contain any 8-bit value, including
    // embedded zeros ('\\0')."
    let stdout = run_ok(
        "io.write(string.char(0, 65, 0, 66))",
        "lumelir_io_write_embedded_nul_preserves",
    );
    assert_eq!(stdout, &[0u8, 65, 0, 66][..]);
}

#[test]
fn io_write_middle_nul_preserves_bytes() {
    // NUL in the middle of the byte sequence — different chokepoint
    // pin from leading-NUL above.
    let stdout = run_ok(
        "io.write(string.char(65, 0, 66))",
        "lumelir_io_write_middle_nul_preserves",
    );
    assert_eq!(stdout, &[65u8, 0, 66][..]);
}

#[test]
fn io_write_lone_nul_preserves_bytes() {
    // 1-byte string containing only NUL: `printf("%s")` would emit
    // zero bytes (truncate immediately); `fwrite(_, 1, 1, _)` emits
    // the byte.
    let stdout = run_ok(
        "io.write(string.char(0))",
        "lumelir_io_write_lone_nul_preserves",
    );
    assert_eq!(stdout, &[0u8][..]);
}

#[test]
fn io_write_ascii_regression() {
    // NUL-free strings must continue to emit identical bytes
    // through the new fwrite path (no off-by-one, no missing
    // terminator). Regression pin for ADR 0117.
    let stdout = run_ok("io.write(\"ABC\")", "lumelir_io_write_ascii_regression");
    assert_eq!(stdout, &[65u8, 66, 67][..]);
}

#[test]
fn io_write_no_trailing_newline() {
    // Two consecutive io.write calls produce concatenated
    // output with no newline between them.
    let stdout = run_ok(
        "io.write(\"a\")\nio.write(\"b\")",
        "lumelir_io_write_no_newline",
    );
    assert_eq!(stdout, b"ab");
}

#[test]
fn io_write_then_print_adds_newline() {
    // io.write writes inline; the subsequent print appends its
    // own trailing newline.
    let stdout = run_ok(
        "io.write(\"a\")\nprint(\"b\")",
        "lumelir_io_write_then_print",
    );
    assert_eq!(stdout, b"ab\n");
}

#[test]
fn io_write_tagged_index_source_dispatches_on_runtime_tag() {
    // Mirrors the Print ADR 0065 fix: when the arg is an inline
    // `Index` expression (here `arg[1]`), the codegen must route
    // through the tagged-tmp materialisation path even when
    // `infer_kind` statically returns Number (HIR does not
    // propagate the table-element kind). Without this dispatch
    // the String payload is read as a Number and traps with
    // "table value type mismatch".
    //
    // This test exercises the combined ADR 0115 (`arg` table) +
    // ADR 0116 (`io.write`) surface end-to-end.
    let path = std::env::temp_dir().join("lumelir_io_write_arg_idx.lua");
    std::fs::write(&path, "io.write(\"hi \"); io.write(arg[1])").expect("write fixture");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_lumelir"))
        .args(["run", path.to_str().unwrap(), "world"])
        .output()
        .expect("spawn lumelir");
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "io.write(arg[1]) must succeed: stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"hi world");
}

// --- HIR negative pins (Lua §6.6: Number or String only) ---

#[test]
fn io_write_rejects_bool() {
    let chunk = lumelir::parser::parse("io.write(true)").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("io.write with Bool must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn io_write_rejects_nil() {
    let chunk = lumelir::parser::parse("io.write(nil)").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("io.write with Nil must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn io_write_rejects_table_literal() {
    // Table literal as arg: Lua spec rejects (only Number/String).
    // The HIR error variant for tables-as-values is shared with
    // other builtins; we accept any HIR error here as long as
    // compile rejects.
    let chunk = lumelir::parser::parse("io.write({})").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("io.write with Table must reject");
    // Any HIR error is acceptable; the test pins the reject
    // behavior, not the exact variant.
    let _ = err;
}

// --- Shadowing positive pin (codex critical) ---

#[test]
fn io_write_shadowed_respects_user_table() {
    // User shadow `local io = {}` must take over per Lua semantics.
    let src = "local io = {}
function io.write(x) return x + 700 end
print(io.write(42))";
    let stdout = run_ok(src, "lumelir_io_write_shadowed");
    assert_eq!(stdout, b"742\n");
}
