//! Phase 2.6+-multi-segment-method-def (ADR 0096): Multi-segment
//! method-def parser delta. Closes the original ADR 0092 carry-over
//! by extending `parse_method_def` to loop over `.IDENT` segments;
//! HIR `lower_method_def` folds receiver_chain into nested Index
//! AST and reuses ADR 0095's TAG_TABLE narrowing infrastructure
//! unmodified.
//!
//! Codex critical fix incorporated: FuncId allocation decoupled
//! from `method_funcs` index registration. ALL MethodDef get
//! FuncIds via Pass-1 `register_method_signature`; `method_funcs`
//! insertion is gated to `receiver_chain.len() == 1` (call-site
//! refinement boundary; ADR 0093/0094 walker matches `Index{Ident, Str}`
//! only). Multi-segment FuncId lookup uses a parallel `Vec<FuncId>`
//! + `methoddef_seq` counter (mirrors `funcdef_seq` pattern).
//!
//! Non-goals (intentional):
//! - Multi-segment method-call refinement (`a.b.c.m(x)` call-site).
//! - Source-order shadowing resolution.
//! - Bare top-level `function NAME() end`.
//! - `self` widen to TaggedValue.

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
fn three_segment_dotted_def_and_call() {
    // 3-segment dotted method-def: `function a.b.c() end`. Receiver
    // chain is ["app", "utils"], method "format". HIR folds to
    // nested Index target → ADR 0095 widen + TAG_TABLE narrow.
    let src = "local app = {}
app.utils = {}
function app.utils.format(x) return x + 1 end
print(app.utils.format(41))";
    assert_eq!(run_ok(src, "lumelir_multi_seg_dotted").trim(), "42");
}

#[test]
fn three_segment_colon_def_compiles() {
    // 3-segment colon method-def: `function a.b:c() end`. Receiver
    // chain ["app", "utils"], method "scale", is_colon=true. body
    // gets implicit `self` param (Table kind per ADR 0092).
    //
    // The colon-call form `app.utils:scale(...)` requires MethodCall
    // with Index receiver (ADR 0092's ComplexMethodReceiver out-of-MVP
    // restriction). The explicit-self form `app.utils.scale(recv, x)`
    // requires call-site multi-segment receiver kind narrowing
    // (TaggedValue→Table at arg position) which is also out of MVP
    // (orthogonal ADR 0091-0094 carry-over).
    //
    // For this ADR, verify the 3-segment colon-DEF compilation path
    // works end-to-end (parser → HIR fold → ADR 0095 widen at
    // IndexAssign → codegen TAG_TABLE narrow). Method body is never
    // invoked; just exits successfully.
    let src = "local app = {}
app.utils = {}
function app.utils:scale(x) return x * 2 end
print(\"defined\")";
    assert_eq!(
        run_ok(src, "lumelir_multi_seg_colon_compile").trim(),
        "defined"
    );
}

#[test]
fn four_segment_boundary_dotted() {
    // 4-segment boundary case to pin parser loop correctness.
    // `function a.b.c.d.method() end` — receiver_chain length 4.
    let src = "local a = {}
a.b = {}
a.b.c = {}
a.b.c.d = {}
function a.b.c.d.compute(x) return x + 10 end
print(a.b.c.d.compute(5))";
    assert_eq!(run_ok(src, "lumelir_multi_seg_4").trim(), "15");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn two_segment_def_unchanged() {
    // ADR 0092's 2-segment path must remain working: `function obj.m()` /
    // `function obj:m()`. The receiver: String → receiver_chain: Vec<String>
    // AST rename produces equivalent length-1 chain; lowering is
    // identical (Ident target, no widen needed).
    let src = "local obj = {}
function obj:add(x) return x + 1 end
print(obj:add(41))";
    assert_eq!(run_ok(src, "lumelir_multi_seg_regress").trim(), "42");
}
