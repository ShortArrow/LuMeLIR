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

// ============================================================================
// ADR 0102 — math.* continuation (pow / sin / cos / log / exp)
// ============================================================================

// --- 5 happy-path tests (Red Day 0, Green after Step 3) ---

#[test]
fn math_pow_basic() {
    // Codex critical: pow is BINARY — its own test face separate
    // from the unary group. 2 ** 10 = 1024.
    let src = "print(math.pow(2, 10))";
    assert_eq!(run_ok(src, "lumelir_math_pow").trim(), "1024");
}

#[test]
fn math_sin_zero() {
    // sin(0) = 0 (exact in f64).
    let src = "print(math.sin(0))";
    assert_eq!(run_ok(src, "lumelir_math_sin").trim(), "0");
}

#[test]
fn math_cos_zero() {
    // cos(0) = 1 (exact in f64).
    let src = "print(math.cos(0))";
    assert_eq!(run_ok(src, "lumelir_math_cos").trim(), "1");
}

#[test]
fn math_log_one() {
    // log(1) = 0 (natural log; exact in f64).
    let src = "print(math.log(1))";
    assert_eq!(run_ok(src, "lumelir_math_log").trim(), "0");
}

#[test]
fn math_exp_zero() {
    // exp(0) = 1 (exact in f64).
    let src = "print(math.exp(0))";
    assert_eq!(run_ok(src, "lumelir_math_exp").trim(), "1");
}

// --- 1 binary-arity pin for math.pow (Codex critical) ---

#[test]
fn math_pow_arity_mismatch() {
    // Codex critical: pin the binary signature. Calling math.pow
    // with 1 arg must surface ArityMismatch (lower_math_builtin_call
    // helper from ADR 0101 enforces builtin.arity()).
    let chunk = lumelir::parser::parse("print(math.pow(2))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("math.pow with 1 arg must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

// --- ADR 0110: arg-kind validation negative pins ---
//
// namespace builtin の per-arg kind は静的に検証される。
// math.* は全て Number を要求; 非 Number は HIR で reject。
// TaggedValue (table-lookup 由来等) は skip — 将来 ADR で
// runtime tag-check を追加。

#[test]
fn math_sqrt_rejects_string() {
    let chunk = lumelir::parser::parse("print(math.sqrt(\"hello\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("math.sqrt with String must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn math_pow_rejects_string_first() {
    let chunk = lumelir::parser::parse("print(math.pow(\"x\", 2))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("math.pow with String first must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn math_floor_rejects_bool() {
    let chunk = lumelir::parser::parse("print(math.floor(true))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("math.floor with Bool must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}
