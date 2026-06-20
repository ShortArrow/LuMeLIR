//! ADR 0234 — M8-C: subtype propagation through HIR expressions
//! in the `propagate_number_subtype` post-pass. Locals receiving
//! Integer+Integer arithmetic (Add/Sub/Mul/FloorDiv/Mod/Bitwise/
//! Shifts), as well as Locals reading from other Integer-subtype
//! Locals, inherit `NumberSubtype::Integer`. Div (`/`) and Pow
//! (`^`) always yield Float; mixed Integer/Float operands yield
//! Float; everything else yields Unknown.

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
fn local_from_int_local_add_keeps_integer() {
    assert_eq!(
        run_ok(
            "local a = 10
local b = a + 5
print(math.type(b))",
            "lumelir_m8c_add"
        )
        .trim(),
        "integer"
    );
}

#[test]
fn local_from_int_local_mul_keeps_integer() {
    assert_eq!(
        run_ok(
            "local a = 6
local b = 7
local c = a * b
print(math.type(c))",
            "lumelir_m8c_mul"
        )
        .trim(),
        "integer"
    );
}

#[test]
fn local_from_int_div_is_float() {
    // `/` is always Float per Lua §3.4.1, even for integer
    // operands.
    assert_eq!(
        run_ok(
            "local a = 10
local b = a / 2
print(math.type(b))",
            "lumelir_m8c_div"
        )
        .trim(),
        "float"
    );
}

#[test]
fn local_from_int_floordiv_keeps_integer() {
    assert_eq!(
        run_ok(
            "local a = 10
local b = a // 3
print(math.type(b))",
            "lumelir_m8c_floordiv"
        )
        .trim(),
        "integer"
    );
}

#[test]
fn local_from_mixed_int_float_becomes_float() {
    assert_eq!(
        run_ok(
            "local a = 10
local b = a + 1.5
print(math.type(b))",
            "lumelir_m8c_mixed"
        )
        .trim(),
        "float"
    );
}

#[test]
fn local_from_int_bitand_keeps_integer() {
    assert_eq!(
        run_ok(
            "local a = 12
local b = a & 7
print(math.type(b))",
            "lumelir_m8c_bitand"
        )
        .trim(),
        "integer"
    );
}

#[test]
fn local_chain_of_int_ops_print_via_lld() {
    // Chained Integer-subtype assignments + final print routes
    // through the ADR 0233 %lld path.
    assert_eq!(
        run_ok(
            "local a = 100
local b = a + 50
local c = b * 2
print(c)",
            "lumelir_m8c_chain_print"
        )
        .trim(),
        "300"
    );
}
