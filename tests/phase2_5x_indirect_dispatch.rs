//! Phase 2.5x-callee-dispatch (ADR 0082): per-call-site static
//! dispatch chain over compatible user functions. The new positive
//! coverage that ADR 0082 enables — multi-return indirect, single
//!   and multi positions for the same callee, and the closure-
//!   escape and tag-mismatch backstops.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(result.status.success(), "binary should exit 0: {result:?}");
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn multi_return_indirect_call_via_tagged_local() {
    // Multi-return user function called through a tagged local.
    // The call site's `local a, b = g(...)` shape causes
    // `lower_local_multi` to filter the dispatch candidate set
    // by `(param_kinds, ret_kinds) = ([Number, Number], [Number,
    // Number])`.
    let src = "local function pair_inc(a, b) return a + 1, b + 1 end
local fns = {pair_inc}
local f = fns[1]
local x, y = f(10, 20)
print(x, y)";
    assert_eq!(run(src, "lumelir_dispatch_multi").trim(), "11\t21");
}

#[test]
fn closure_with_upvalues_via_indirect_table_now_runs() {
    // ADR 0083 Commit 3c (supersedes 0044/0071's closure-escape
    // reject for table values): a capturing closure can now flow
    // into a table-element slot, then back out via `t[i]` /
    // `local g = t[1]`, and the dispatch chain through the
    // tagged-slot loaded ptr still finds the closure's user fn
    // — but goes through the cell-ptr-first ABI path so the
    // captured `outer` upvalue is reached via the cell.
    let src = "local outer = 5
local function clo(x) return x + outer end
local t = {clo}
local g = t[1]
print(g(10))";
    assert_eq!(run(src, "lumelir_dispatch_clo_in_tbl").trim(), "15");
}

#[test]
fn no_compatible_user_fn_is_compile_error() {
    // ADR 0082: when no user function in the module matches the
    // call site's `(param_kinds, ret_kinds)` signature, the
    // dispatch candidate set is empty and lowering reports
    // `IndirectCallNoCandidates` rather than emitting a runtime
    // trap. The runtime-trap path is reserved for future FFI /
    // dynamic-loader producers.
    //
    // Here the table holds a 0-arg function, but the call site
    // passes 1 arg. No user fn matches `(Number,) → Number`.
    let chunk = lumelir::parser::parse(
        "local function zero_arg() return 0 end
local t = {zero_arg}
local g = t[1]
print(g(42))",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("expected NoCandidates HIR error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("IndirectCallNoCandidates"),
        "expected IndirectCallNoCandidates, got: {msg}"
    );
}

#[test]
fn dispatch_selects_correct_function_among_two_same_signature() {
    // Two user functions sharing `(Number,) → Number`. The runtime
    // ptr-equality chain selects the right one based on the actual
    // value loaded from the slot.
    let src = "local function double(x) return x * 2 end
local function half(x) return x / 2 end
local t = {double, half}
local a = t[1]
local b = t[2]
print(a(7))
print(b(8))";
    assert_eq!(run(src, "lumelir_dispatch_two_sigs").trim(), "14\n4");
}
