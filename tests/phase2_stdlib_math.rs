//! Phase 2.7q-stdlib-math (ADR 0101): stdlib math.* builtins
//! (math.sqrt / math.floor / math.abs). First stdlib addition since
//! the original print/tostring/tonumber/type/assert/error/next set;
//! establishes the pattern for future stdlib expansion.
//!
//! Codex post-0100 review (6 視点) critical fixes incorporated:
//! - builtin dispatch only when `math` is UNRESOLVED identifier
//!   (user shadowing `local math = ...` MUST respect the user's
//!   table per Lua semantics).
//! - Test plan: 3 happy + 1 regression-pin + 1 shadowing positive
//!   pin + 1 unknown-method negative pin.
//!
//! Non-goals (intentional):
//! - Other math.* functions (math.pi / sin / cos / log / pow etc.).
//! - string.* / table.* / io.* libraries.
//! - `local m = math; m.sqrt(x)` aliasing.
//! - `math` as a first-class chunk-scope table.

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

// --- 3 happy-path tests ---

#[test]
fn math_sqrt_basic() {
    let src = "print(math.sqrt(16))";
    assert_eq!(run_ok(src, "lumelir_math_sqrt").trim(), "4");
}

#[test]
fn math_floor_basic() {
    let src = "print(math.floor(3.7))";
    assert_eq!(run_ok(src, "lumelir_math_floor").trim(), "3");
}

#[test]
fn math_abs_negative_arg() {
    let src = "print(math.abs(-5))
print(math.abs(5))";
    let stdout = run_ok(src, "lumelir_math_abs");
    let lines: Vec<&str> = stdout.trim().split('\n').collect();
    assert_eq!(lines, vec!["5", "5"]);
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn existing_ident_call_unchanged() {
    // The existing print + arithmetic path must remain working
    // after the new math.* builtin dispatch is added.
    let src = "local function double(n) return n * 2 end
print(double(21))";
    assert_eq!(run_ok(src, "lumelir_math_regression").trim(), "42");
}

// --- 1 shadowing positive pin (Codex critical) ---

#[test]
fn user_shadowed_math_respects_user_table() {
    // Codex critical: if the user declares `local math = ...`,
    // the `math.*(...)` call must dispatch via the user's table
    // (NOT the math.* builtin). `resolve("math").is_none()` guard
    // bypasses the builtin path when math is a declared local.
    //
    // Setup: user defines `math.identity(x) = x + 100`, calls it.
    // If the builtin dispatch incorrectly fires for `math.identity`,
    // it would either fail (no identity builtin) or fire the wrong
    // function. With the shadowing guard, the call resolves
    // through the user's table → identity returns 142.
    let src = "local math = {}
function math.identity(x) return x + 100 end
print(math.identity(42))";
    assert_eq!(run_ok(src, "lumelir_math_shadowed").trim(), "142");
}

// --- 1 unknown-method negative pin (Codex critical) ---

#[test]
fn math_unknown_method_not_builtin() {
    // Codex critical: `math.unknown(x)` (unknown method) must NOT
    // silently dispatch to some default builtin. Falls through to
    // the normal Index-callee path, which fails because `math` is
    // unresolved at HIR time. Surfaces as a typed error (UnknownFunction
    // or UndefinedName).
    let chunk = lumelir::parser::parse("print(math.notarealmath(4))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("unknown math method must fail");
    let msg = format!("{err:?}");
    // Allow either UndefinedName or UnknownFunction — both are
    // acceptable surface errors for an unresolved math.unknown.
    assert!(
        msg.contains("UndefinedName") || msg.contains("UnknownFunction"),
        "expected typed UndefinedName/UnknownFunction error, got: {msg}"
    );
}
