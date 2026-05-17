//! Phase 2.7r-stdlib-table (ADR 0106): table.* library begin via
//! `table.concat(t)` — the first non-math, non-string consumer of
//! ADR 0103's `Builtin::from_namespace_method` generic dispatcher.
//!
//! Option A scope (Codex post-0105 critical): arity 1 only,
//! implicit `sep=""`. The `table.concat(t, sep)` (arity 2) and
//! `table.concat(t, sep, i, j)` (arity 3-4) forms are deferred
//! to future ADRs. Strict Number-or-String element trap per
//! Lua 5.4 §6.8 spec.
//!
//! Non-goals (documented in ADR 0106):
//! - sep / i / j args
//! - table.insert / remove / unpack / sort / pack / move
//! - Builtin::arity() range refactor
//! - malloc OOM null-check, fptosi NaN guards
//! - Hash part walk (Lua spec: array part only)

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

// --- 4 happy-path tests ---

#[test]
fn table_concat_basic_strings() {
    let src = "print(table.concat({\"a\", \"b\", \"c\"}))";
    assert_eq!(run_ok(src, "lumelir_table_concat_str").trim(), "abc");
}

#[test]
fn table_concat_basic_numbers() {
    // Number elements are stringified via emit_tostring Number arm
    // (snprintf %.14g path) per ADR 0026.
    let src = "print(table.concat({1, 2, 3}))";
    assert_eq!(run_ok(src, "lumelir_table_concat_num").trim(), "123");
}

#[test]
fn table_concat_mixed_num_str() {
    // Tag-dispatch in both passes: TAG_NUMBER → stringify,
    // TAG_STRING → payload ptr directly.
    let src = "print(table.concat({1, \"x\", 2}))";
    assert_eq!(run_ok(src, "lumelir_table_concat_mixed").trim(), "1x2");
}

#[test]
fn table_concat_single_element() {
    let src = "print(table.concat({\"only\"}))";
    assert_eq!(run_ok(src, "lumelir_table_concat_single").trim(), "only");
}

// --- 1 boundary test ---

#[test]
fn table_concat_empty_table() {
    // length == 0 → emit_empty_string() (ADR 0104 helper reuse).
    let src = "print(table.concat({}))";
    assert_eq!(run_ok(src, "lumelir_table_concat_empty").trim(), "");
}

// --- 1 trap pin (Codex critical) ---

#[test]
fn table_concat_bool_element_traps() {
    // Lua spec §6.8: non-Number/non-String elements raise runtime
    // error. We trap via `s_table_concat_bad_element` diagnostic
    // global. Test only asserts non-zero exit (not stderr content)
    // to stay loosely coupled to the message wording.
    let src = "print(table.concat({true}))";
    let out = compile_and_run(src, "lumelir_table_concat_bool_trap");
    assert!(
        !out.status.success(),
        "bool element must trap, but binary exited 0: {out:?}"
    );
}

// --- 1 shadowing positive pin (Codex critical) ---

#[test]
fn table_concat_shadowed_respects_user_table() {
    // Same shadowing semantics as math.* / string.* (ADR
    // 0101/0103). `local table = {}` shadows the namespace; the
    // user's `table.concat` is called.
    let src = "local table = {}
function table.concat(x) return x + 500 end
print(table.concat(42))";
    assert_eq!(run_ok(src, "lumelir_table_concat_shadowed").trim(), "542");
}

// --- 1 arity pin (Codex critical) ---

#[test]
fn table_concat_arity_zero_fails() {
    // Arity range (1, 2); calling with 0 must surface ArityMismatch
    // via `lower_namespace_builtin_call` uniform range check after
    // ADR 0107 (was: fixed arity 1 with special case).
    let chunk = lumelir::parser::parse("print(table.concat())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

// --- ADR 0107: table.concat(t, sep) — 5 happy + 1 arity-3 pin
// + 1 dynamic-sep happy ---
//
// Lua 5.4 §6.8 arity 2 form: insert `sep` between adjacent
// elements (no leading/trailing sep). Empty `sep` is a no-op,
// equivalent to arity 1. Single-element table emits no sep
// at all (no adjacency to insert into).

#[test]
fn table_concat_with_sep_basic() {
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \", \"))";
    assert_eq!(
        run_ok(src, "lumelir_table_concat_sep_basic").trim(),
        "a, b, c"
    );
}

#[test]
fn table_concat_with_empty_sep() {
    // Explicit "" sep ≡ arity 1 default. Pins arity-2 dispatch
    // path returning the same bytes as the arity-1 path.
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \"\"))";
    assert_eq!(run_ok(src, "lumelir_table_concat_sep_empty").trim(), "abc");
}

#[test]
fn table_concat_empty_with_sep() {
    // Empty table → "" regardless of sep (no adjacencies).
    let src = "print(table.concat({}, \", \"))";
    assert_eq!(run_ok(src, "lumelir_table_concat_empty_sep").trim(), "");
}

#[test]
fn table_concat_single_with_sep() {
    // 1-element table → element only, no sep prefix/suffix.
    let src = "print(table.concat({\"only\"}, \", \"))";
    assert_eq!(
        run_ok(src, "lumelir_table_concat_single_sep").trim(),
        "only"
    );
}

#[test]
fn table_concat_numbers_with_sep() {
    // Number elements pass through emit_tostring(Number) +
    // sep insertion between them.
    let src = "print(table.concat({1, 2, 3}, \"-\"))";
    assert_eq!(run_ok(src, "lumelir_table_concat_num_sep").trim(), "1-2-3");
}

#[test]
fn table_concat_with_dynamic_sep() {
    // sep need not be a literal; any String-kind expression
    // works. Pins that emit_expr for args[1] integrates with
    // the codegen sep-insertion path.
    let src = "local s = \", \"
print(table.concat({\"a\", \"b\"}, s))";
    assert_eq!(
        run_ok(src, "lumelir_table_concat_dynamic_sep").trim(),
        "a, b"
    );
}

// --- ADR 0108: table.concat(t, sep, i, j) full Lua 5.4 §6.8 ---
//
// Bounds normalization reuses ADR 0104's `emit_normalize_sub_bounds`
// helper verbatim (cross-namespace reuse — the architectural payoff
// of this ADR). i/j semantics match `string.sub`: negative-translate,
// clamp to [1, #t], i > j returns empty string.
//
// Default i = 1; default j = #t. Lua spec mandates strict
// out-of-bounds error but we follow `string.sub`'s clamping precedent
// (deliberate spec deviation; arg-validation policy ADR may revisit).

#[test]
fn table_concat_with_i_only() {
    // Default j = #t (= 3).
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \"-\", 2))";
    assert_eq!(run_ok(src, "lumelir_table_concat_i_only").trim(), "b-c");
}

#[test]
fn table_concat_with_i_and_j() {
    // Explicit bounds.
    let src = "print(table.concat({\"a\", \"b\", \"c\", \"d\"}, \"-\", 2, 3))";
    assert_eq!(run_ok(src, "lumelir_table_concat_i_and_j").trim(), "b-c");
}

#[test]
fn table_concat_negative_i() {
    // Negative i → #t + i + 1. -2 → 3 - 2 + 1 = 2 → walk [2, 3].
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \"-\", -2))";
    assert_eq!(run_ok(src, "lumelir_table_concat_neg_i").trim(), "b-c");
}

#[test]
fn table_concat_negative_j() {
    // Negative j → #t + j + 1. -2 → 4 - 2 + 1 = 3 → walk [1, 3].
    let src = "print(table.concat({\"a\", \"b\", \"c\", \"d\"}, \"-\", 1, -2))";
    assert_eq!(run_ok(src, "lumelir_table_concat_neg_j").trim(), "a-b-c");
}

#[test]
fn table_concat_i_equals_j() {
    // Single-element range; no sep insertion.
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \"-\", 2, 2))";
    assert_eq!(run_ok(src, "lumelir_table_concat_i_eq_j").trim(), "b");
}

#[test]
fn table_concat_i_greater_j_empty() {
    // i > j after normalize → empty string (outer guard).
    let src = "print(table.concat({\"a\", \"b\", \"c\"}, \"-\", 3, 1))";
    assert_eq!(run_ok(src, "lumelir_table_concat_i_gt_j").trim(), "");
}

#[test]
fn table_concat_j_clamp_to_len() {
    // j > #t → clamp to #t. String.sub precedent (Lua spec
    // technically mandates error; deliberate clamp deviation).
    let src = "print(table.concat({\"a\", \"b\"}, \"-\", 1, 100))";
    assert_eq!(run_ok(src, "lumelir_table_concat_j_clamp").trim(), "a-b");
}

#[test]
fn table_concat_arity_five_fails() {
    // Range is (1, 4); 5 args must reject. Pins the new upper
    // bound after the (1, 2) → (1, 4) widening.
    let chunk = lumelir::parser::parse("print(table.concat({}, \",\", 1, 2, 3))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with 5 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}

// --- ADR 0110: arg-kind validation negative pins ---
//
// table.concat の per-arg kind を静的検証。
// arg 0 Table, arg 1 String, args 2/3 Number。
// TaggedValue 由来 (table-lookup 由来等) は skip — 将来 ADR で
// runtime tag-check を追加。

#[test]
fn table_concat_rejects_non_table() {
    let chunk = lumelir::parser::parse("print(table.concat(\"not_table\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with String must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn table_concat_rejects_non_string_sep() {
    let chunk = lumelir::parser::parse("print(table.concat({}, 42))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with Number sep must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn table_concat_rejects_non_number_i() {
    let chunk = lumelir::parser::parse("print(table.concat({}, \",\", \"x\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with String i must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}

#[test]
fn table_concat_rejects_non_number_j() {
    let chunk = lumelir::parser::parse("print(table.concat({}, \",\", 1, \"y\"))").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with String j must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("BuiltinArgKindMismatch"),
        "expected BuiltinArgKindMismatch, got: {msg}"
    );
}
