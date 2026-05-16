//! Phase 2.7q-stdlib-string (ADR 0103): string.* library begin.
//! Adds string.len + string.upper + string.lower; refactors the
//! `lower_call` dispatch from math-hardcoded to namespace-generic
//! (codex post-0102 critical) so future ADRs adding table.* /
//! io.* etc. only extend `Builtin::from_namespace_method`.
//!
//! Codex post-0102 review (6 視点) critical fixes incorporated:
//! - `try_namespace_builtin` generic dispatch NOW (NOT
//!   string-also-hardcode for future cleanup).
//! - `emit_string_case_map(op_name)` helper for upper/lower
//!   shared malloc + memcpy + scf::for char-loop logic.
//! - 6 tests: 3 happy + shadowing positive + unknown-method
//!   negative + arity pin.
//! - Separate AGENTS row `2.7q-stdlib-string` (not extending math).
//!
//! Non-goals (intentional):
//! - string.sub / format / rep / find / match / byte / char /
//!   reverse — future incremental ADRs.
//! - `s:len()` method syntax (needs metatable).
//! - UTF-8 / multi-byte char handling.
//! - malloc OOM null-check (carry-over from concat / closure /
//!   table; future ADR consolidates).

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
fn string_len_basic() {
    let src = "print(string.len(\"hello\"))";
    assert_eq!(run_ok(src, "lumelir_string_len").trim(), "5");
}

#[test]
fn string_upper_basic() {
    let src = "print(string.upper(\"abc\"))";
    assert_eq!(run_ok(src, "lumelir_string_upper").trim(), "ABC");
}

#[test]
fn string_lower_basic() {
    let src = "print(string.lower(\"XYZ\"))";
    assert_eq!(run_ok(src, "lumelir_string_lower").trim(), "xyz");
}

// --- 1 shadowing positive pin (Codex critical) ---

#[test]
fn string_shadowed_respects_user_table() {
    // Same shadowing-respect semantics as math (ADR 0101). If
    // user declares `local string = {}`, the namespace builtin
    // dispatch must skip and let the user's table take over.
    let src = "local string = {}
function string.identity(x) return x + 100 end
print(string.identity(42))";
    assert_eq!(run_ok(src, "lumelir_string_shadowed").trim(), "142");
}

// --- 1 unknown-method negative pin (Codex critical) ---

#[test]
fn string_unknown_method_not_builtin() {
    // `string.unknown(x)` (unrecognized method) must NOT silently
    // dispatch to some default builtin. Falls through to the
    // normal Index-callee path → UndefinedName for unresolved
    // `string`.
    let chunk = lumelir::parser::parse("print(string.notarealfn(\"x\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("unknown string method must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("UndefinedName") || msg.contains("UnknownFunction"),
        "expected typed UndefinedName/UnknownFunction error, got: {msg}"
    );
}

// --- 1 arity pin (Codex critical) ---

#[test]
fn string_len_arity_mismatch() {
    // string.len takes exactly 1 arg. Calling with 0 must surface
    // ArityMismatch (via lower_namespace_builtin_call from ADR
    // 0101 helper).
    let chunk = lumelir::parser::parse("print(string.len())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.len with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}
