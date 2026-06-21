//! ADR 0239 — M10-B: pin `__mode` metatable-field capability +
//! document the weak-table runtime deferral. `__mode = "k"`,
//! `"v"`, `"kv"` can be stored on a metatable and round-tripped
//! as a normal String field. Lua 5.4 §2.5.4 spec semantics
//! (weak keys / weak values cleared at GC time) are gated on
//! M3-extended Table-GC tracking and live in a future
//! sub-ADR.

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
fn mode_k_string_can_be_stored() {
    assert_eq!(
        run_ok(
            "local mt = { __mode = \"k\" }
print(mt.__mode)",
            "lumelir_m10_mode_k"
        )
        .trim(),
        "k"
    );
}

#[test]
fn mode_v_string_can_be_stored() {
    assert_eq!(
        run_ok(
            "local mt = { __mode = \"v\" }
print(mt.__mode)",
            "lumelir_m10_mode_v"
        )
        .trim(),
        "v"
    );
}

#[test]
fn mode_kv_string_can_be_stored() {
    assert_eq!(
        run_ok(
            "local mt = { __mode = \"kv\" }
print(mt.__mode)",
            "lumelir_m10_mode_kv"
        )
        .trim(),
        "kv"
    );
}

#[test]
fn setmetatable_with_mode_does_not_error() {
    // Pin that attaching a `__mode` metatable parses + compiles
    // + runs. ADR 0239 §Scope documents the weak-table
    // semantics as deferred runtime work.
    assert_eq!(
        run_ok(
            "local mt = { __mode = \"k\" }
local cache = setmetatable({}, mt)
print(\"ok\")",
            "lumelir_m10_mode_setmt"
        )
        .trim(),
        "ok"
    );
}

#[test]
fn mode_field_is_just_a_string_field_today() {
    // Bridges the contract: `__mode` is currently a plain
    // string-valued field with no GC-time interpretation.
    // Round-trip via concat proves the storage shape.
    assert_eq!(
        run_ok(
            "local mt = { __mode = \"kv\" }
print(\"mode=\" .. mt.__mode)",
            "lumelir_m10_mode_concat"
        )
        .trim(),
        "mode=kv"
    );
}
