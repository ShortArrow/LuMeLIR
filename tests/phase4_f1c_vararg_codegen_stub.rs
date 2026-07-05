//! ADR 0295 — F1-C-step1: stub codegen for `...` that evaluates to
//! a literal nil. This unblocks the end-to-end compile path so
//! vararg function bodies referencing `...` no longer error out.
//! Actual pack materialisation (call-site spread + hidden param +
//! multi-return propagation) is F1-C-step2/step3.

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
fn vararg_body_compiles_and_runs() {
    // Bare `local t = ...` inside a vararg fn now compiles.
    let out = run_ok(
        "local function f(...) local t = ... end
f()
print(\"ok\")",
        "f1c_bare",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn vararg_no_extras_yields_nil() {
    // With zero extras, `_va_pack` is an empty Table and the
    // Index nil-on-missing path yields nil — matches Lua.
    let out = run_ok(
        "local function f(...) local t = ... print(type(t)) end
f()",
        "f1c_type_nil",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn vararg_table_ctor_counts_extras() {
    // ADR 0298 — `{...}` aliases the pack; `#` reports extras count.
    let out = run_ok(
        "local function count(...) print(#{...}) end
count(10, 20, 30)",
        "f1c_pack_count",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn vararg_pack_forwarding_passes_all() {
    // ADR 0298 — `inner(...)` forwards the whole pack, not just
    // the first extra.
    let out = run_ok(
        "local function fwd(...)
    local function inner(...) print(#{...}) end
    inner(...)
end
fwd(1, 2, 3, 4)",
        "f1c_forwarding",
    );
    assert_eq!(out.trim(), "4");
}

#[test]
fn vararg_pack_elements_readable_by_index() {
    // Pack elements are ordinary table reads.
    let out = run_ok(
        "local function f(...)
    local t = {...}
    print(t[1])
    print(t[3])
end
f(7, 8, 9)",
        "f1c_pack_index",
    );
    let lines: Vec<&str> = out.trim().split('\n').collect();
    assert_eq!(lines, vec!["7", "9"]);
}

#[test]
fn vararg_stub_with_extra_call_args() {
    // ADR 0296 step2 landed — extras are packed into `_va_pack`
    // and `...` in single-value position reads `_va_pack[1]`.
    let out = run_ok(
        "local function f(...) local t = ... print(type(t)) end
f(1, 2, 3)",
        "f1c_type_extras",
    );
    assert_eq!(out.trim(), "number");
}

#[test]
fn nested_vararg_functions_compile() {
    // step2: outer(1, 2) packs (1, 2) into outer's _va_pack;
    // outer's `inner(...)` unpacks `...` to `1` (first extra) then
    // calls inner(1); inner's _va_pack gets [1]; `t = ...` reads 1.
    let out = run_ok(
        "local function outer(...)
    local function inner(...)
        local t = ...
        return t
    end
    return inner(...)
end
print(type(outer(1, 2)))",
        "f1c_nested",
    );
    assert_eq!(out.trim(), "number");
}
