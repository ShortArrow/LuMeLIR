//! ADR 0214 — `print(HirExprKind::Integer)` emits `%lld` printf so
//! static integer args preserve full i64 precision instead of being
//! demoted via `sitofp` + `%g`. Also drops the f64 fraction artifact
//! (`12` instead of `12.0`).

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
fn print_small_integer_has_no_fraction() {
    assert_eq!(run_ok("print(42)", "lumelir_print_int_small").trim(), "42");
}

#[test]
fn print_maxinteger_preserves_precision() {
    assert_eq!(
        run_ok("print(math.maxinteger)", "lumelir_print_maxint").trim(),
        "9223372036854775807"
    );
}

#[test]
fn print_mininteger_preserves_precision() {
    assert_eq!(
        run_ok("print(math.mininteger)", "lumelir_print_minint").trim(),
        "-9223372036854775808"
    );
}

#[test]
fn print_folded_integer_binop_has_no_fraction() {
    assert_eq!(run_ok("print(1 + 2)", "lumelir_print_int_add").trim(), "3");
}

#[test]
fn print_multiple_integer_args_tab_separated() {
    assert_eq!(
        run_ok("print(1, 2, 3)", "lumelir_print_int_multi").trim(),
        "1\t2\t3"
    );
}
