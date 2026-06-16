//! ADR 0213 — Integer + Integer constant folding at HIR. Preserves
//! the Integer subtype through static arithmetic so `math.type(1 + 2)`
//! returns "integer". Mixed Integer/Float and overflow cases fall
//! through to the f64 path.

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
fn add_integer_integer_folds_to_integer() {
    assert_eq!(
        run_ok("print(math.type(1 + 2))", "lumelir_fold_add").trim(),
        "integer"
    );
}

#[test]
fn sub_integer_integer_folds_to_integer() {
    assert_eq!(
        run_ok("print(math.type(5 - 3))", "lumelir_fold_sub").trim(),
        "integer"
    );
}

#[test]
fn mul_integer_integer_folds_to_integer() {
    assert_eq!(
        run_ok("print(math.type(4 * 3))", "lumelir_fold_mul").trim(),
        "integer"
    );
}

#[test]
fn floordiv_integer_integer_folds_to_integer() {
    assert_eq!(
        run_ok("print(math.tointeger(10 // 3))", "lumelir_fold_floordiv").trim(),
        "3"
    );
}

#[test]
fn bitand_integer_integer_folds_to_integer() {
    assert_eq!(
        run_ok("print(math.type(7 & 5))", "lumelir_fold_bitand").trim(),
        "integer"
    );
}

#[test]
fn mixed_integer_float_does_not_fold_to_integer() {
    // 1 + 2.0 has a Float operand; result subtype is float (or lost
    // to nil in Phase B static-shape detection). Must NOT be integer.
    assert_ne!(
        run_ok("print(math.type(1 + 2.0))", "lumelir_fold_mixed").trim(),
        "integer"
    );
}
