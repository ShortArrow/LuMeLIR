//! Phase 2.6+-method-idx-call-refine (ADR 0094): Index-callee Call
//! arg refinement + helper extraction. Closes the orthogonal ADR 0091
//! / ADR 0093 carry-over where dotted MethodDef called via Index-callee
//! Call (`t.helper(arg)`) doesn't refine the registered method's param
//! kinds, causing IndirectCallNoCandidates at the dispatcher.
//!
//! Fix shape (Codex post-0093 critical fix incorporated):
//! - Extract `try_refine_func_args(idx, base, args, kinds, seen)` helper
//!   in `infer_user_function_param_kinds` so the three refinement arms
//!   (Ident-Call, MethodCall, Index-callee Call) share the kinds/seen
//!   update body. Only FuncId lookup + arg base index differ.
//! - Index-callee refinement fires inside the existing `Call` arm as a
//!   secondary if-let when `callee = Index { target: Ident, key: Str }`,
//!   reusing the ADR 0093 `method_funcs` index with base=0.
//!
//! Non-goals (intentional):
//! - Index target / key non-literal (`(get_obj()).m(x)`, `t[k](x)`).
//! - Function-kind upvalue refinement.
//! - Source-order shadowing resolution.
//! - `self` kind refinement (stays Table per ADR 0092 policy; the
//!   colon-def + explicit-self call kinds[idx][0] refinement is a
//!   no-op because `lower_method_def` re-seeds Table at the
//!   for_function call site).

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

// --- 2 happy-path tests (Red Day 0, Green after Step 2) ---

#[test]
fn dotted_def_index_callee_string_arg_refines() {
    // The natural pattern that ADR 0094 closes: dotted method-def
    // called via Index-callee Call with a String literal. Before
    // ADR 0094, `name` defaults to Number; dispatcher reports
    // IndirectCallNoCandidates { param_kinds: [String] }.
    let src = "local t = {}
function t.helper(name) return \"hello \" .. name end
print(t.helper(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_idx_callee_string").trim(),
        "hello world"
    );
}

#[test]
fn colon_def_explicit_self_call_refines() {
    // Colon method-def called via explicit-self Index-callee form
    // `t.m(t, x)`. args=[t, x]; ADR 0094 refinement with base=0
    // sets kinds[idx][0]=Table (from t) and kinds[idx][1]=String
    // (from "world"). Index-0 refinement is a no-op because
    // `lower_method_def` re-seeds external_kinds[0]=Table per
    // ADR 0092 policy. Index-1 is the material refinement that
    // unblocks String arg dispatch.
    let src = "local obj = {}
function obj:greet(name) return \"hi \" .. name end
print(obj.greet(obj, \"there\"))";
    assert_eq!(
        run_ok(src, "lumelir_idx_callee_explicit_self").trim(),
        "hi there"
    );
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn methodcall_path_unchanged_after_helper_extract() {
    // ADR 0093's MethodCall refinement path (colon-def + colon-call)
    // must remain working after Step 1's helper extract refactor.
    // Same shape as one of ADR 0093's tests; pinned here so a
    // regression in the helper extract path surfaces locally.
    let src = "local obj = {}
function obj:greet(name) return \"hello \" .. name end
print(obj:greet(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_methodcall_path_pin").trim(),
        "hello world"
    );
}
