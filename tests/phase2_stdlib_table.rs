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
    // Fixed arity 1; calling with 0 must surface ArityMismatch via
    // `lower_namespace_builtin_call`.
    let chunk = lumelir::parser::parse("print(table.concat())").unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("table.concat with 0 args must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ArityMismatch"),
        "expected ArityMismatch, got: {msg}"
    );
}
