//! ADR 0211 — `math.tointeger(x)` static-shape integer-valued check.

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
fn math_tointeger_integer_literal_returns_value() {
    assert_eq!(
        run_ok("print(math.tointeger(42))", "lumelir_mti_int").trim(),
        "42"
    );
}

#[test]
fn math_tointeger_integer_valued_float_returns_value() {
    // 42.0 is integer-valued (fract == 0); returns 42.
    assert_eq!(
        run_ok("print(math.tointeger(42.0))", "lumelir_mti_fzero").trim(),
        "42"
    );
}

#[test]
fn math_tointeger_fractional_returns_nil() {
    assert_eq!(
        run_ok("print(math.tointeger(42.5))", "lumelir_mti_frac").trim(),
        "nil"
    );
}

#[test]
fn math_tointeger_local_returns_nil_phase_b() {
    // Phase B: subtype info lost at Local declaration.
    // ADR 0212+ tracks subtype through tagged slots.
    assert_eq!(
        run_ok(
            "local x = 42; print(math.tointeger(x))",
            "lumelir_mti_local"
        )
        .trim(),
        "nil"
    );
}
