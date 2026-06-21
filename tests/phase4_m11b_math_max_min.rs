//! ADR 0241 — M11-B: variadic `math.max` / `math.min`. Lowered
//! as a left-to-right reduce over `arith.maximumf` /
//! `arith.minimumf`.

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
fn max_two_args() {
    assert_eq!(
        run_ok("print(math.max(3, 7))", "lumelir_m11b_max_2").trim(),
        "7"
    );
}

#[test]
fn min_two_args() {
    assert_eq!(
        run_ok("print(math.min(3, 7))", "lumelir_m11b_min_2").trim(),
        "3"
    );
}

#[test]
fn max_single_arg_is_identity() {
    assert_eq!(
        run_ok("print(math.max(42))", "lumelir_m11b_max_1").trim(),
        "42"
    );
}

#[test]
fn max_four_args_left_to_right_reduce() {
    assert_eq!(
        run_ok("print(math.max(1, 5, 3, 9))", "lumelir_m11b_max_4").trim(),
        "9"
    );
}

#[test]
fn min_with_negatives() {
    assert_eq!(
        run_ok("print(math.min(-2, -5, -1))", "lumelir_m11b_min_neg").trim(),
        "-5"
    );
}

#[test]
fn max_with_fractional() {
    assert_eq!(
        run_ok("print(math.max(3.2, 3.7, 3.5))", "lumelir_m11b_max_frac").trim(),
        "3.7"
    );
}

#[test]
fn max_composes_with_other_math() {
    // Composes with math.sqrt + math.abs.
    assert_eq!(
        run_ok(
            "print(math.max(math.abs(-3), math.sqrt(4)))",
            "lumelir_m11b_max_compose"
        )
        .trim(),
        "3"
    );
}
