//! ADR 0232 — M8-A: runtime Local subtype tracking via
//! `LocalInfo::subtype` (Integer / Float / Unknown). A post-pass
//! over the HIR walks every LocalInit / Assign and records the
//! RHS shape. `math.type(Local)` reads it to return
//! "integer" / "float" / nil.

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
fn local_integer_literal_classifies_as_integer() {
    assert_eq!(
        run_ok(
            "local x = 42
print(math.type(x))",
            "lumelir_m8a_int_local"
        )
        .trim(),
        "integer"
    );
}

#[test]
fn local_float_literal_classifies_as_float() {
    assert_eq!(
        run_ok(
            "local x = 3.14
print(math.type(x))",
            "lumelir_m8a_float_local"
        )
        .trim(),
        "float"
    );
}

#[test]
fn local_arithmetic_result_is_unknown() {
    // Locals that receive a non-literal RHS (Call result, BinOp
    // result through a runtime variable, etc.) remain Unknown
    // subtype — math.type returns nil.
    assert_eq!(
        run_ok(
            "local x = tonumber(\"5\")
print(math.type(x))",
            "lumelir_m8a_call_local"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn local_reassigned_to_different_subtype_becomes_unknown() {
    // A local initially set to Integer then reassigned to Float
    // merges to Unknown subtype.
    assert_eq!(
        run_ok(
            "local x = 1
x = 2.5
print(math.type(x))",
            "lumelir_m8a_mixed_local"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn local_consistent_integer_assignments_stay_integer() {
    // Multiple Integer LocalInit / Assign keep the subtype at
    // Integer (the post-pass's merge semantics).
    assert_eq!(
        run_ok(
            "local x = 1
x = 2
x = 3
print(math.type(x))",
            "lumelir_m8a_consistent_int"
        )
        .trim(),
        "integer"
    );
}
