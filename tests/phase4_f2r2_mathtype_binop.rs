//! ADR 0301 — F2-R2: math.type over BinOp results.

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
fn div_is_always_float() {
    let out = run_ok("print(math.type(10 / 4))", "f2r2_div");
    assert_eq!(out.trim(), "float");
}

#[test]
fn pow_is_always_float() {
    let out = run_ok("print(math.type(2^2))", "f2r2_pow");
    assert_eq!(out.trim(), "float");
}

#[test]
fn int_plus_int_is_integer() {
    let out = run_ok("print(math.type(1 + 2))", "f2r2_intadd");
    assert_eq!(out.trim(), "integer");
}

#[test]
fn int_plus_float_is_float() {
    let out = run_ok("print(math.type(1 + 2.0))", "f2r2_mixed");
    assert_eq!(out.trim(), "float");
}

#[test]
fn floordiv_of_ints_is_integer() {
    let out = run_ok("print(math.type(7 // 2))", "f2r2_floordiv");
    assert_eq!(out.trim(), "integer");
}

#[test]
fn local_subtype_flows_through_binop() {
    let out = run_ok(
        "local x = 5
print(math.type(x * 2))",
        "f2r2_local_flow",
    );
    assert_eq!(out.trim(), "integer");
}
