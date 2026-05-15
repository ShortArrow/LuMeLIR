//! Phase 2.6+-callee-norm (ADR 0091, v2 post-abort): HIR callee
//! normalization for Index-callee Calls. Closes the gap that ADR 0091
//! plan v1 ("methods") unintentionally exposed — namely that
//! `lower_call` (`src/hir/mod.rs:3613-3619`) rejects any non-Ident
//! callee with `UnsupportedCall`, breaking `obj.method(args)` and
//! `t[i](args)` direct-call forms before any sugar layer can sit on
//! top.
//!
//! Fix shape (per codex post-abort review):
//! - HIR pre-stmt hoisting infrastructure (`LowerCtx::pending_pre_stmts`
//!   drained at every `lower_stmt` boundary).
//! - Pure `classify_callee_form` decides DirectIdent vs IndexCallee.
//! - Effectful `materialize_callee_to_local` lowers the Index, declares
//!   a synthetic TaggedValue local (`__callee_<seq>`), pushes a
//!   `LocalInit` pre-stmt, and routes the call through the existing
//!   `Callee::IndirectDispatch` machinery (ADR 0082, LocalId-source
//!   invariant preserved).
//!
//! Non-goals (out of MVP per codex):
//! - Method colon syntax (`obj:method()`) — future ADR depends on this.
//! - `self` param-kind refinement.
//! - `infer_user_function_param_kinds` extension for Index-callee Calls.
//! - New `Callee` variants.

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

// --- 3 happy-path tests (Red Day 0, Green after Step 4) ---

#[test]
fn index_field_callee_dispatches() {
    // `t.m(args)` direct-call form. Without ADR 0091, HIR rejects
    // with UnsupportedCall before codegen sees anything. After
    // ADR 0091, the Index result is pre-bound to a synthetic local
    // and routed through the existing ADR 0082 IndirectDispatch
    // chain.
    let src = "local t = {}
t.m = function(x) return x + 1 end
print(t.m(2))";
    assert_eq!(run_ok(src, "lumelir_idx_field_call").trim(), "3");
}

#[test]
fn index_numeric_callee_dispatches() {
    // Numeric-key Index-callee. `arr[1](args)`.
    let src = "local arr = {}
arr[1] = function(x) return x * 10 end
print(arr[1](5))";
    assert_eq!(run_ok(src, "lumelir_idx_num_call").trim(), "50");
}

#[test]
fn index_callee_body_arith_works() {
    // Arith inside the called function body must still work after
    // routing through Index-callee → synth-local → IndirectDispatch.
    // Confirms the synth-local pre-binding doesn't disturb function
    // body lowering or downstream MLIR codegen.
    let src = "local t = {}
t.compute = function(x) return (x + 1) * 2 end
print(t.compute(3))";
    assert_eq!(run_ok(src, "lumelir_idx_body_arith").trim(), "8");
}

// --- 1 regression-pin (always green) ---

#[test]
fn existing_local_binding_unchanged() {
    // The `local g = t.m; g(args)` pattern already works via
    // ADR 0082 IndirectDispatch. ADR 0091 must preserve this
    // behavior — the synth-local hoist is a NEW path that
    // augments, not replaces, the existing flow.
    let src = "local t = {}
t.m = function(x) return x + 1 end
local g = t.m
print(g(2))";
    assert_eq!(run_ok(src, "lumelir_idx_localbind_regress").trim(), "3");
}

// --- 2 error pins ---

#[test]
fn index_callee_no_candidates_reports_typed_error() {
    // No user function in the module matches the Index-callee
    // Call's signature (Number,) → Number. ADR 0091 must route
    // through IndirectDispatch's candidate filter (existing ADR
    // 0082 machinery), surfacing IndirectCallNoCandidates rather
    // than the generic UnsupportedCall.
    let chunk = lumelir::parser::parse(
        "local function zero_arg() return 0 end
local t = {}
t[1] = zero_arg
print(t[1](42))",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("expected typed HIR error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("IndirectCallNoCandidates"),
        "expected IndirectCallNoCandidates, got: {msg}"
    );
}

#[test]
fn index_callee_on_non_function_traps_at_runtime() {
    // `t[1]` is a Number value (not Function). ADR 0091 lowers
    // through synth-local + IndirectDispatch, whose runtime tag
    // check sees TAG_NUMBER ≠ TAG_FUNCTION and exits with
    // `s_call_non_function` (ADR 0082). A dummy user function
    // provides a candidate so HIR doesn't reject at compile time
    // with IndirectCallNoCandidates.
    let src = "local function dummy(x) return x end
local t = {1, 2, 3}
print(t[1](5))";
    let output = std::env::temp_dir().join("lumelir_idx_nonfn_trap");
    let chunk = lumelir::parser::parse(src).expect("parse");
    let hir = lumelir::hir::lower(&chunk).expect("HIR lower should succeed");
    lumelir::codegen::compile(&hir, &output).expect("codegen");
    let result = Command::new(&output)
        .output()
        .expect("failed to run binary");
    let _ = std::fs::remove_file(&output);
    assert!(
        !result.status.success(),
        "binary should trap on non-Function callee"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    // ADR 0082 trap text:
    assert!(
        combined.contains("call") && combined.contains("function"),
        "expected non-function call trap message; got: {combined}"
    );
}
