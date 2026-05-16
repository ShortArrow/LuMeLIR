//! Phase 2.6+-nested-index-assign-widen (ADR 0095): nested
//! IndexAssign target widening with TAG_TABLE runtime narrow.
//! Closes the chokepoint that ADR 0092's multi-segment method-def
//! carry-over required.
//!
//! Today's flow rejects `app.utils.field = 10` at HIR with a
//! TypeMismatch [Table, Number]: the nested `app.utils` lowers as
//! `HirExprKind::Index` whose `infer_kind` returns Number; the
//! IndexAssign target_kind check requires Table.
//!
//! Fix shape (codex-style chokepoint resolution):
//! - HIR: `widen_index_for_assign_target` rewrites Index → IndexTagged
//!   at IndexAssign target position (mirrors ADR 0063
//!   `widen_index_for_local_init` idempotent shape). target_kind check
//!   loosens to accept TaggedValue in addition to Table.
//! - Codegen: emit_stmt's IndexAssign arm gains an IndexTagged target
//!   branch that performs a TAG_TABLE check + extracts the table
//!   descriptor at runtime; traps via new s_index_assign_target_not_table
//!   on tag mismatch.
//!
//! Non-goals:
//! - Multi-segment method-def parser support (ADR 0096 follow-up).
//! - Index-read kind narrowing in general expression positions.
//! - Source-order shadowing resolution.

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

// --- 3 happy-path tests (Red Day 0, Green after Step 3) ---

#[test]
fn nested_field_assign_then_read() {
    // The exact chokepoint case: `app.utils.field = 10`. The
    // IndexAssign target is `Index{Ident(app), Str("utils")}`,
    // requiring TaggedValue-target support after the widen.
    let src = "local app = {}
app.utils = {}
app.utils.field = 10
print(app.utils.field)";
    assert_eq!(run_ok(src, "lumelir_nested_field").trim(), "10");
}

#[test]
fn nested_array_assign_then_read() {
    // Number-key variant: `app.utils[1] = 10`. Target shape is
    // the same `Index{Ident, Str}` (key = "utils" is String);
    // the inner write's key kind is Number (array path). The
    // widen applies to the target position regardless of the
    // write's key kind.
    let src = "local app = {}
app.utils = {}
app.utils[1] = 42
print(app.utils[1])";
    assert_eq!(run_ok(src, "lumelir_nested_array").trim(), "42");
}

#[test]
fn nested_assign_overwrites() {
    // Verify the new path writes correctly when called twice:
    // the second write must overwrite the first. Catches any
    // bug where the TAG_TABLE narrow re-reads stale descriptor.
    let src = "local app = {}
app.utils = {}
app.utils.value = 1
app.utils.value = 99
print(app.utils.value)";
    assert_eq!(run_ok(src, "lumelir_nested_overwrite").trim(), "99");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn single_level_assign_unchanged() {
    // ADR 0055's single-level IndexAssign path (target is a
    // Table-kind local) must remain working. The widen is
    // idempotent on non-Index targets (Local stays Local),
    // so this path is unchanged.
    let src = "local t = {}
t.field = 7
print(t.field)";
    assert_eq!(run_ok(src, "lumelir_single_level_pin").trim(), "7");
}
