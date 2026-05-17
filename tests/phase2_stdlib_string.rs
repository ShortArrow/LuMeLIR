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

// --- ADR 0104: string.sub (5 happy + 3 boundary + 2 arity + 1 shadow) ---
//
// Lua 5.4 §6.4 semantics:
//   string.sub(s, i [, j])
//     - i < 0 → from-end (#s + i + 1)
//     - i < 1 after translate → clamped to 1
//     - j absent → j = #s
//     - j < 0 → from-end (#s + j + 1)
//     - j > #s → clamped to #s
//     - i > j after normalize → empty string
//     - else → bytes s[i-1..j-1] (0-indexed, inclusive)
//
// Bounds-normalization helper is extracted as pure SSA logic
// (Codex critical pre-extract). String-slice helper is
// future-reusable for `string.find` capture extraction.

#[test]
fn string_sub_basic_2arg() {
    let src = "print(string.sub(\"hello\", 2, 4))";
    assert_eq!(run_ok(src, "lumelir_string_sub_basic").trim(), "ell");
}

#[test]
fn string_sub_suffix_negative_i() {
    let src = "print(string.sub(\"hello\", -3))";
    assert_eq!(run_ok(src, "lumelir_string_sub_suffix").trim(), "llo");
}

#[test]
fn string_sub_prefix_2arg() {
    let src = "print(string.sub(\"hello\", 1, 3))";
    assert_eq!(run_ok(src, "lumelir_string_sub_prefix").trim(), "hel");
}

#[test]
fn string_sub_all_omit_j() {
    let src = "print(string.sub(\"hello\", 1))";
    assert_eq!(run_ok(src, "lumelir_string_sub_all").trim(), "hello");
}

#[test]
fn string_sub_negative_j() {
    let src = "print(string.sub(\"hello\", 2, -1))";
    assert_eq!(run_ok(src, "lumelir_string_sub_neg_j").trim(), "ello");
}

#[test]
fn string_sub_j_clamp_to_len() {
    let src = "print(string.sub(\"abc\", 1, 100))";
    assert_eq!(run_ok(src, "lumelir_string_sub_clamp_j").trim(), "abc");
}

#[test]
fn string_sub_i_past_end_empty() {
    let src = "print(string.sub(\"abc\", 10))";
    assert_eq!(run_ok(src, "lumelir_string_sub_past_end").trim(), "");
}

#[test]
fn string_sub_i_gt_j_empty() {
    // After normalization i=3, j=1 → i > j → empty.
    let src = "print(string.sub(\"hello\", 3, 1))";
    assert_eq!(run_ok(src, "lumelir_string_sub_i_gt_j").trim(), "");
}

#[test]
fn string_sub_arity_zero_fails() {
    // string.sub takes 2 or 3 args; 0 must surface ArityMismatch.
    let chunk = lumelir::parser::parse("print(string.sub())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.sub with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_sub_arity_four_fails() {
    // string.sub takes 2 or 3 args; 4 must surface ArityMismatch.
    let chunk = lumelir::parser::parse("print(string.sub(\"a\", 1, 2, 3))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.sub with 4 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_sub_shadowed_respects_user_table() {
    // Same shadowing semantics as string.len (ADR 0103). If
    // user declares `local string = {}`, the namespace builtin
    // dispatch must skip and let the user's table take over.
    let src = "local string = {}
function string.sub(x) return x + 200 end
print(string.sub(42))";
    assert_eq!(run_ok(src, "lumelir_string_sub_shadowed").trim(), "242");
}
