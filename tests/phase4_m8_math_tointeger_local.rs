//! ADR 0235 — M8-D: `math.tointeger(Local-Integer)` returns the
//! Integer value. Reads the slot's f64 and returns via the
//! TaggedValue Number arm (same success path as the static
//! Integer literal). Locals without Integer subtype fall
//! through to Nil.

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
fn tointeger_local_integer_returns_value() {
    assert_eq!(
        run_ok(
            "local x = 42
print(math.tointeger(x))",
            "lumelir_m8d_tointeger_local"
        )
        .trim(),
        "42"
    );
}

#[test]
fn tointeger_local_after_binop_returns_value() {
    // Composes with ADR 0234 BinOp propagation.
    assert_eq!(
        run_ok(
            "local a = 5
local b = a * 7
print(math.tointeger(b))",
            "lumelir_m8d_tointeger_binop"
        )
        .trim(),
        "35"
    );
}

#[test]
fn tointeger_local_float_returns_nil() {
    assert_eq!(
        run_ok(
            "local x = 3.14
print(math.tointeger(x))",
            "lumelir_m8d_tointeger_float"
        )
        .trim(),
        "nil"
    );
}

#[test]
fn tointeger_local_unknown_subtype_returns_nil() {
    // tonumber Call result → Unknown subtype → Nil.
    assert_eq!(
        run_ok(
            "local x = tonumber(\"7\")
print(math.tointeger(x))",
            "lumelir_m8d_tointeger_unknown"
        )
        .trim(),
        "nil"
    );
}
