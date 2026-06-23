//! ADR 0261 — N3-C: userdata via `newproxy(b)` Builtin.
//!
//! Pure-Lua source has no spec-blessed userdata creation API (the
//! standard lib relies on the C FFI). This ADR implements Lua's
//! de-facto `newproxy` (deprecated 5.0 onwards but commonly
//! emulated): allocates a GC-tracked TAG_USERDATA value with an
//! arbitrary 16-byte payload. The arg is accepted but ignored
//! (spec `b` flag for metatable inheritance is not yet modeled).
//!
//! Establishes the type-system plumbing for any future FFI /
//! runtime-bridge Builtin that may yield userdata. `type(ud)`
//! correctly returns `"userdata"`; the value round-trips through
//! Locals.

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
fn newproxy_returns_userdata_type() {
    let out = run_ok(
        "local ud = newproxy(false)
print(type(ud))",
        "lumelir_n3c_newproxy_type",
    );
    assert_eq!(out.trim(), "userdata");
}

#[test]
fn two_newproxy_calls_each_report_userdata() {
    // Each call allocates a fresh GC object; both report the same
    // type ("userdata"). TAG_USERDATA equality is a future ADR
    // (requires extending the eq-engine with a new tag arm).
    let out = run_ok(
        "local a = newproxy(false)
local b = newproxy(false)
print(type(a))
print(type(b))",
        "lumelir_n3c_newproxy_two",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "userdata");
    assert_eq!(lines[1], "userdata");
}

#[test]
fn newproxy_value_survives_collectgarbage_if_rooted() {
    let out = run_ok(
        "local ud = newproxy(false)
collectgarbage()
print(type(ud))",
        "lumelir_n3c_newproxy_rooted",
    );
    assert_eq!(out.trim(), "userdata");
}
