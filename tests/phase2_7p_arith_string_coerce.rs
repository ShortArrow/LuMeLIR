//! Phase 2.7p-arith-string-coerce (ADR 0077): Lua spec §3.4.1
//! string→number arithmetic coercion. Adds runtime
//! `tonumber`-style parsing of String operands in arithmetic /
//! bitwise BinOps. Failure (`"abc" + 1`) traps at runtime
//! rather than silently propagating NaN — distinguishing this
//! path from the `tonumber` builtin (ADR 0028) whose failure
//! returns the NaN sentinel.
//!
//! Out of scope (separate LIC entries):
//! - TaggedValue operand coerce — HIR can't statically resolve
//!   the runtime kind; LIC-2.7p-arith-coerce-tagged-1.
//! - Comparison (`<`, `<=`, `>`, `>=`) string-number mixing is
//!   a Lua-spec error; not in this phase.
//!
//! Hex floats (`"0x10" + 0`) work today via glibc's
//! `sscanf("%lf")` — pinned by `string_arith_hex_works_via_
//! glibc_sscanf` below.

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

fn run(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0, got {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ============================================================
// Positive coercion
// ============================================================

#[test]
fn string_plus_number_decimal() {
    assert_eq!(run("print(\"5\" + 1)", "lumelir_coerce_dec").trim(), "6");
}

#[test]
fn string_times_number_float() {
    assert_eq!(
        run("print(\"3.14\" * 2)", "lumelir_coerce_float").trim(),
        "6.28"
    );
}

#[test]
fn string_minus_number_negative() {
    assert_eq!(
        run("print(\"-2.5\" + 1)", "lumelir_coerce_neg").trim(),
        "-1.5"
    );
}

#[test]
fn string_plus_number_scientific() {
    assert_eq!(
        run("print(\"1e3\" + 0)", "lumelir_coerce_sci").trim(),
        "1000"
    );
}

#[test]
fn string_op_string_both_coerce() {
    assert_eq!(
        run("print(\"3\" + \"4\")", "lumelir_coerce_both").trim(),
        "7"
    );
}

#[test]
fn string_div_number_coerce() {
    assert_eq!(run("print(\"10\" / 4)", "lumelir_coerce_div").trim(), "2.5");
}

// ============================================================
// Negative — runtime trap on parse failure
// ============================================================

#[test]
fn string_arith_invalid_input_traps() {
    let out = compile_and_run("print(\"abc\" + 1)", "lumelir_coerce_trap_abc");
    assert!(
        !out.status.success(),
        "unparseable string in arith must trap, got exit 0"
    );
}

#[test]
fn string_arith_hex_works_via_glibc_sscanf() {
    // glibc's `sscanf("%lf")` accepts hex floats including the
    // `0x` prefix, so `"0x10" + 0` parses to 16 matching Lua
    // spec. This test pins the behaviour so we notice if the
    // host libc changes. Other libc implementations may need a
    // custom parser; tracked outside this phase.
    assert_eq!(
        run("print(\"0x10\" + 0)", "lumelir_coerce_hex").trim(),
        "16"
    );
}

// ============================================================
// Regression — pure-Number arith and tonumber NaN unchanged
// ============================================================

#[test]
fn regression_pure_number_arith_unchanged() {
    assert_eq!(run("print(2 + 3)", "lumelir_coerce_reg_num").trim(), "5");
}

#[test]
fn regression_tonumber_failure_still_returns_nan() {
    // ADR 0028 contract: tonumber("oops") returns the NaN sentinel
    // (observable via `x == x` flip). ADR 0077 must not break this.
    let src = "local x = tonumber(\"oops\")
if x == x then print(\"finite\") else print(\"nan\") end";
    assert_eq!(run(src, "lumelir_coerce_reg_tonumber_nan").trim(), "nan");
}
