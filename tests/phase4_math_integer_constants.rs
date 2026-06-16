//! ADR 0212 — `math.maxinteger` / `math.mininteger` Integer
//! constants. Lower to HirExprKind::Integer so math.type returns
//! "integer". Phase B print loses precision via f64 demotion;
//! Phase C will preserve precision via %lld.

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
fn math_maxinteger_has_integer_subtype() {
    assert_eq!(
        run_ok("print(math.type(math.maxinteger))", "lumelir_maxint_type").trim(),
        "integer"
    );
}

#[test]
fn math_mininteger_has_integer_subtype() {
    assert_eq!(
        run_ok("print(math.type(math.mininteger))", "lumelir_minint_type").trim(),
        "integer"
    );
}

#[test]
fn math_tointeger_on_maxinteger_returns_value_or_nil() {
    // Phase B demotion to f64: i64::MAX loses precision. The
    // tointeger check on the demoted value passes is_finite and
    // fract == 0 so returns the (imprecise) Number. Phase C will
    // distinguish.
    let out = run_ok(
        "print(math.tointeger(math.maxinteger))",
        "lumelir_mti_maxint",
    );
    assert!(
        !out.trim().is_empty() && out.trim() != "nil",
        "expected a Number result (Phase B demotion), got: {out:?}"
    );
}
