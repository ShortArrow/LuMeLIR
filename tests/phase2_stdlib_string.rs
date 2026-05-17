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

// --- ADR 0105: string.rep (1 happy + 4 boundary + 2 arity + 1 shadow) ---
//
// Lua 5.4 §6.4 semantics:
//   string.rep(s, n)
//     - n <= 0 → empty string
//     - n == 1 → s (copy)
//     - n >= 1 → buf = malloc(n * #s + 1); n × memcpy(buf+i*#s, s, #s)
//
// Fixed arity 2; the variadic `sep` form `string.rep(s, n, sep)` is
// a future ADR.

#[test]
fn string_rep_basic() {
    let src = "print(string.rep(\"ab\", 3))";
    assert_eq!(run_ok(src, "lumelir_string_rep_basic").trim(), "ababab");
}

#[test]
fn string_rep_zero_is_empty() {
    // n = 0 → "" (Lua spec).
    let src = "print(string.rep(\"ab\", 0))";
    assert_eq!(run_ok(src, "lumelir_string_rep_zero").trim(), "");
}

#[test]
fn string_rep_one_is_identity_copy() {
    // n = 1 → single copy, identical bytes.
    let src = "print(string.rep(\"ab\", 1))";
    assert_eq!(run_ok(src, "lumelir_string_rep_one").trim(), "ab");
}

#[test]
fn string_rep_two_doubles() {
    // n = 2 → loop runs twice.
    let src = "print(string.rep(\"ab\", 2))";
    assert_eq!(run_ok(src, "lumelir_string_rep_two").trim(), "abab");
}

#[test]
fn string_rep_empty_src_is_empty() {
    // Empty src × any n → empty (n * 0 = 0).
    let src = "print(string.rep(\"\", 5))";
    assert_eq!(run_ok(src, "lumelir_string_rep_empty_src").trim(), "");
}

#[test]
fn string_rep_negative_n_is_empty() {
    // n < 0 → "" per Lua spec (count_pos branch absorbs).
    let src = "print(string.rep(\"ab\", -1))";
    assert_eq!(run_ok(src, "lumelir_string_rep_neg").trim(), "");
}

#[test]
fn string_rep_arity_zero_fails() {
    let chunk = lumelir::parser::parse("print(string.rep())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.rep with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_rep_arity_three_fails() {
    // Fixed arity 2; the variadic `sep` form is a future ADR.
    let chunk = lumelir::parser::parse("print(string.rep(\"a\", 2, \"x\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.rep with 3 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_rep_shadowed_respects_user_table() {
    let src = "local string = {}
function string.rep(x) return x + 300 end
print(string.rep(42))";
    assert_eq!(run_ok(src, "lumelir_string_rep_shadowed").trim(), "342");
}

// --- ADR 0109: string.byte(s, i?) — 3rd consumer of
// emit_normalize_sub_bounds (ADR 0104) ---
//
// Lua 5.4 §6.4 single-position form. Returns the byte code
// (0-255) at index i (default 1). Negative i indexes from the
// end. Out-of-range traps (Lua spec returns nil; we trap because
// Number-return contract has no nil representation).
//
// emit_normalize_sub_bounds is reused with j_raw = i_raw — the
// 3rd call shape (single-position) after string.sub (range slice)
// and table.concat (join walk). Validates the abstraction across
// 3 distinct consumers.

#[test]
fn string_byte_default_i() {
    let src = "print(string.byte(\"ABC\"))";
    assert_eq!(run_ok(src, "lumelir_string_byte_default").trim(), "65");
}

#[test]
fn string_byte_explicit_i() {
    let src = "print(string.byte(\"ABC\", 2))";
    assert_eq!(run_ok(src, "lumelir_string_byte_explicit").trim(), "66");
}

#[test]
fn string_byte_negative_i_last() {
    // i = -1 → from-end last char (C = 67).
    let src = "print(string.byte(\"ABC\", -1))";
    assert_eq!(run_ok(src, "lumelir_string_byte_neg_last").trim(), "67");
}

#[test]
fn string_byte_negative_i_first() {
    // i = -3 → from-end first char (A = 65).
    let src = "print(string.byte(\"ABC\", -3))";
    assert_eq!(run_ok(src, "lumelir_string_byte_neg_first").trim(), "65");
}

#[test]
fn string_byte_single_char() {
    // 1-char string, default i=1.
    let src = "print(string.byte(\"a\"))";
    assert_eq!(run_ok(src, "lumelir_string_byte_single").trim(), "97");
}

#[test]
fn string_byte_out_of_range_traps() {
    // i past end (i=10 on "ABC" len=3): Lua spec returns nil;
    // we trap because Number-return contract has no nil
    // representation. Test only asserts non-zero exit (message
    // wording loosely coupled).
    let src = "print(string.byte(\"ABC\", 10))";
    let out = compile_and_run(src, "lumelir_string_byte_oor_trap");
    assert!(
        !out.status.success(),
        "out-of-range i must trap, but binary exited 0: {out:?}"
    );
}

#[test]
fn string_byte_arity_zero_fails() {
    // arity (1, 2): 0 args must reject via uniform range check.
    let chunk = lumelir::parser::parse("print(string.byte())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.byte with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_byte_arity_three_fails() {
    // Single-position only; multi-byte form (s, i, j) is a future
    // multi-result-builtin ADR. 3 args reject at the (1, 2) upper
    // bound.
    let chunk = lumelir::parser::parse("print(string.byte(\"a\", 1, 2))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.byte with 3 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

#[test]
fn string_byte_shadowed_respects_user_table() {
    let src = "local string = {}
function string.byte(x) return x + 400 end
print(string.byte(42))";
    assert_eq!(run_ok(src, "lumelir_string_byte_shadowed").trim(), "442");
}

// --- ADR 0110: arg-kind validation negative pins ---
//
// string.* の per-arg kind を静的検証。
// String が必要な引数に Number/Bool/Nil 等を渡すと HIR reject。
// TaggedValue は skip (runtime check 将来 ADR)。

#[test]
fn string_len_rejects_number() {
    let chunk = lumelir::parser::parse("print(string.len(42))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.len with Number must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn string_upper_rejects_nil() {
    let chunk = lumelir::parser::parse("print(string.upper(nil))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.upper with Nil must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn string_sub_rejects_non_string_first() {
    let chunk = lumelir::parser::parse("print(string.sub(42, 1))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.sub with Number first must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn string_sub_rejects_non_number_second() {
    let chunk = lumelir::parser::parse("print(string.sub(\"a\", \"x\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.sub with String second must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn string_byte_rejects_non_string() {
    let chunk = lumelir::parser::parse("print(string.byte(123))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("string.byte with Number must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}
