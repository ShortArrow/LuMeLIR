//! Phase 3.0-rust-lua-bridge-mvp (ADR 0191): minimum-viable
//! Rust-Lua Bridge. `rust.add(a: Number, b: Number) -> Number`
//! dispatches through `Builtin::from_namespace_method` to the
//! `extern "C" fn rust_add` symbol provided by the bundled
//! `src/bridge_runtime.rs` object, linked into every compiled
//! binary via `build.rs` + `option_env!("LUMELIR_BRIDGE_OBJ")`.

use std::process::Command;

fn compile_and_run(src: &str, output_name: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    let _ = std::fs::remove_file(&output);
    result
}

fn run_ok(src: &str, output_name: &str) -> String {
    let out = compile_and_run(src, output_name);
    assert!(out.status.success(), "binary should exit 0: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- Test 1: Basic call — Rust function returns the sum ---

#[test]
fn rust_add_basic_call() {
    let src = "print(rust.add(2.5, 3.5))";
    assert_eq!(run_ok(src, "lumelir_rust_add_basic").trim(), "6.0");
}

// --- Test 2: Shadowing positive pin (Codex critical, ADR 0103 precedent) ---

#[test]
fn rust_namespace_shadowed_respects_user_table() {
    // Same shadowing-respect semantics as math / string / table.
    // If user declares `local rust = {}`, the namespace bridge
    // dispatch must skip and let the user's table take over.
    let src = "local rust = {}
function rust.add(x, y) return x * y end
print(rust.add(3, 4))";
    assert_eq!(run_ok(src, "lumelir_rust_add_shadowed").trim(), "12");
}

// --- Test 3: Arity pin (Codex critical) ---

#[test]
fn rust_add_arity_zero_fails() {
    // rust.add takes exactly 2 args. Calling with 0 must surface
    // ArityMismatch (via lower_namespace_builtin_call from
    // ADR 0101 helper, same path as math / string / table).
    let chunk = lumelir::parser::parse("print(rust.add())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("rust.add with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}
