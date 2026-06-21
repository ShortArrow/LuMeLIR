//! ADR 0238 — M10-A: pin existing `__gc` metatable-field
//! capability + document the runtime-dispatch deferral. `__gc`
//! can be set as a Function-valued metatable field, retrieved
//! via the metatable, and passed as a value through normal
//! Table machinery. The runtime "fire on unreachable" dispatch
//! is gated on M3-extended (Tables-as-chunk-roots) and lives in
//! a future sub-ADR.

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
fn gc_function_can_be_stored_in_metatable() {
    // The mt itself is a normal Table; __gc as a field name has
    // no special parser handling — it's just a string key.
    assert_eq!(
        run_ok(
            "local mt = { __gc = function(t) return 1 end }
print(type(mt.__gc))",
            "lumelir_m10_gc_field_type"
        )
        .trim(),
        "function"
    );
}

#[test]
fn setmetatable_with_gc_does_not_error() {
    // Attaching a metatable that carries `__gc` succeeds today
    // even though the runtime dispatch is not yet wired. The
    // Lua-spec contract for `__gc` is "called when t becomes
    // unreachable"; the M3 chunk-safe predicate excludes
    // Tables, so Tables are never freed today. ADR 0238 §Scope
    // documents this as a deferred runtime guarantee.
    assert_eq!(
        run_ok(
            "local mt = { __gc = function(t) return 1 end }
local t = setmetatable({}, mt)
print(\"ok\")",
            "lumelir_m10_gc_setmt"
        )
        .trim(),
        "ok"
    );
}

#[test]
fn gc_field_persists_across_multiple_setmetatable() {
    // Distinct tables can share the same mt carrying `__gc`.
    // Pin that re-using the mt doesn't lose the field.
    assert_eq!(
        run_ok(
            "local mt = { __gc = function(t) return 1 end }
local a = setmetatable({}, mt)
local b = setmetatable({}, mt)
print(type(mt.__gc))",
            "lumelir_m10_gc_shared_mt"
        )
        .trim(),
        "function"
    );
}
