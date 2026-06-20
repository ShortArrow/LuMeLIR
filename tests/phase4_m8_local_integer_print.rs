//! ADR 0233 — M8-B: `print(Local-Integer)` + `tostring(Local-
//! Integer)` precision via the `%lld` fast path. The Local's
//! Integer subtype (M8-A) gates the dispatch; the slot's f64
//! is fptosi'd to i64 before formatting so values within ±2^53
//! print without fractional artifacts.

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
fn print_local_integer_uses_lld_format() {
    // `local i = 42` carries Integer subtype; print routes
    // through the %lld fast path. Without M8-B the output went
    // through %g (still prints "42" but loses precision for
    // big values, see next test).
    assert_eq!(
        run_ok(
            "local i = 42
print(i)",
            "lumelir_m8b_print_int"
        )
        .trim(),
        "42"
    );
}

#[test]
fn print_local_large_integer_preserves_precision() {
    // 1_000_000_000_000 = 10^12 — exactly representable in f64.
    // %g would print "1e+12"; %lld prints the full decimal.
    assert_eq!(
        run_ok(
            "local big = 1000000000000
print(big)",
            "lumelir_m8b_print_bigint"
        )
        .trim(),
        "1000000000000"
    );
}

#[test]
fn tostring_local_integer_returns_integer_decimal() {
    assert_eq!(
        run_ok(
            "local n = 12345
print(tostring(n))",
            "lumelir_m8b_tostring_int"
        )
        .trim(),
        "12345"
    );
}

#[test]
fn tostring_local_integer_then_concat() {
    // Concat boxed-string output from tostring(Local-Integer)
    // with a literal — proves the boxed-object shape is sound.
    assert_eq!(
        run_ok(
            "local n = 99
print(\"n=\" .. tostring(n))",
            "lumelir_m8b_tostring_concat"
        )
        .trim(),
        "n=99"
    );
}

#[test]
fn print_local_float_still_uses_g_format() {
    // Sanity: Float subtype Local keeps the %g path
    // (no fractional → integer reshape).
    assert_eq!(
        run_ok(
            "local f = 3.5
print(f)",
            "lumelir_m8b_print_float"
        )
        .trim(),
        "3.5"
    );
}
