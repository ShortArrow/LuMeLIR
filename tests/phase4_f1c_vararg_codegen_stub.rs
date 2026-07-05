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
fn vararg_evaluates_to_nil_in_stub() {
    // Stub semantics: `...` → nil. type() reports "nil".
    let out = run_ok(
        "local function f(...) local t = ... print(type(t)) end
f()",
        "f1c_type_nil",
    );
    assert_eq!(out.trim(), "nil");
}

#[test]
fn vararg_stub_with_extra_call_args() {
    // Even with extras passed at call site, the stub still returns
    // nil — documented deviation until F1-C-step2/3.
    let out = run_ok(
        "local function f(...) local t = ... print(type(t)) end
f(1, 2, 3)",
        "f1c_type_extras",
    );
    // With extras, real Lua returns Number for `t`; our stub still
    // says "nil" and that's the deviation we're pinning.
    assert_eq!(out.trim(), "nil");
}

#[test]
fn nested_vararg_functions_compile() {
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
    assert_eq!(out.trim(), "nil");
}
