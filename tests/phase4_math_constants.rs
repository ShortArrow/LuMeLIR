//! ADR 0208 — `math.pi` / `math.huge` constants recognised by
//! the HIR Index lowering before the namespace-Table resolver
//! rejects `math` as an undefined ident.

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
fn math_pi_prints_approximation() {
    assert_eq!(
        run_ok("print(math.pi)", "lumelir_math_pi").trim(),
        "3.14159"
    );
}

#[test]
fn math_huge_prints_inf() {
    assert_eq!(
        run_ok("print(math.huge)", "lumelir_math_huge").trim(),
        "inf"
    );
}

#[test]
fn math_pi_used_in_arithmetic() {
    // Sanity: math.pi * 2 ≈ 6.28319.
    assert_eq!(
        run_ok("print(math.pi * 2)", "lumelir_math_pi_mul").trim(),
        "6.28319"
    );
}
