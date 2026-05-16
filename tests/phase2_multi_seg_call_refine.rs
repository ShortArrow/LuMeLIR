//! Phase 2.6+-multi-seg-call-refine (ADR 0097): Multi-segment
//! method-call refinement via chain-keyed `method_funcs`
//! unification. Closes the ADR 0091/0094/0096 collective carry-over
//! for the dotted multi-segment call path.
//!
//! Codex post-ADR-0096 review (6 視点) critical fix: unify
//! `method_funcs` from `HashMap<(String, String), FuncId>` to
//! `HashMap<(Vec<String>, String), FuncId>`. Single-segment uses
//! length-1 chain key. One lookup rule, one source of truth — no
//! separate single-seg + multi-seg indices.
//!
//! Non-goals (intentional):
//! - Multi-segment colon-call (`a.b.c:m(x)` MethodCall with Index
//!   receiver — ADR 0092 ComplexMethodReceiver boundary).
//! - Receiver kind narrowing for explicit-self form
//!   (`a.b.c.scale(a.b.c, x)`) — paired future ADR with dispatcher
//!   widening.
//! - Source-order shadowing resolution (last-wins per `function_names`).
//! - `self` widen to TaggedValue.
//! - Non-Ident chain head (`(get_obj()).field.m(x)`) — `extract_index_chain`
//!   returns None; walker safe-skips.

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

// --- 2 happy-path tests (Red Day 0, Green after Step 3) ---

#[test]
fn three_segment_dotted_call_string_arg_refines() {
    // The exact ADR 0096 smoke that surfaced this carry-over:
    // `function app.utils.format(name) end` (3-seg dotted-def)
    // called via `app.utils.format("world")` with String arg.
    // Without ADR 0097, walker doesn't extract the chain
    // ["app", "utils"]/"format" and `name` defaults to Number
    // → IndirectCallNoCandidates. With ADR 0097, chain extraction
    // hits `method_funcs[(["app", "utils"], "format")]` → refine
    // `name = String` → dispatch matches.
    let src = "local app = {}
app.utils = {}
function app.utils.format(name) return \"hello \" .. name end
print(app.utils.format(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_multi_seg_call_3").trim(),
        "hello world"
    );
}

#[test]
fn four_segment_dotted_call_string_arg_refines() {
    // 4-segment boundary case to pin extract_index_chain loop
    // correctness across the longest receiver path.
    let src = "local a = {}
a.b = {}
a.b.c = {}
a.b.c.d = {}
function a.b.c.d.format(name) return \"hi \" .. name end
print(a.b.c.d.format(\"there\"))";
    assert_eq!(run_ok(src, "lumelir_multi_seg_call_4").trim(), "hi there");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn single_seg_refinement_path_unchanged() {
    // ADR 0093/0094's single-segment dotted-def + Index-callee
    // call refinement path must remain working after the
    // chain-keyed unification. Length-1 chain `(["t"], "helper")`
    // must produce the same FuncId mapping as the prior
    // `("t", "helper")` String-keyed lookup.
    let src = "local t = {}
function t.helper(name) return \"hello \" .. name end
print(t.helper(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_single_seg_regress").trim(),
        "hello world"
    );
}
