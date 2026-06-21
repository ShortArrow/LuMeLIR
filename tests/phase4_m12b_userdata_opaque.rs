//! ADR 0244 — M12-B: userdata-as-opaque-Number shim pin. Until
//! a real userdata `ValueKind` lands (M3-extended GC tracking +
//! tag bit), Rust crates that need to return an opaque handle
//! to Lua can encode the pointer as a Number (i64 reinterpreted
//! via the Phase B f64 demotion). Round-tripping through Lua
//! arithmetic loses precision past ±2^53, so this is documented
//! as a known limitation.

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
fn bridge_returns_number_handle_round_trips() {
    // The bridge's rust.add returns Number; treating that Number
    // as an opaque handle (just store + read back via a Local)
    // works through the existing Phase B demotion.
    assert_eq!(
        run_ok(
            "local h = rust.add(3, 4)
print(h)",
            "lumelir_m12b_opaque_roundtrip"
        )
        .trim(),
        "7"
    );
}

#[test]
fn bridge_handle_can_be_passed_back_through_bridge() {
    // The opaque-handle shape: pass a Number through one bridge
    // call into another. rust.add(rust.add(...)) proves the
    // call composition.
    assert_eq!(
        run_ok(
            "print(rust.add(rust.add(1, 2), 3))",
            "lumelir_m12b_chained_bridge"
        )
        .trim(),
        "6"
    );
}

#[test]
fn bridge_handle_arithmetic_preserves_value() {
    // Doubling a Number returned from the bridge — verifies the
    // value flows through Lua arithmetic without loss for small
    // values. (Print uses %g format which loses precision past
    // ~6 significant digits; this test stays small.)
    assert_eq!(
        run_ok(
            "local h = rust.add(42, 0)
print(h * 2)",
            "lumelir_m12b_handle_arith"
        )
        .trim(),
        "84"
    );
}
