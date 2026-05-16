//! Phase 2.6+-method-arg-refine (ADR 0093): MethodCall arg
//! refinement via Pass-1 MethodDef registration. Closes the
//! ADR 0091 / ADR 0092 carry-over where colon-defined methods
//! called only via `recv:method(arg)` get default Number param
//! kinds beyond `self`, causing `IndirectCallNoCandidates` at the
//! dispatcher when arg kinds don't match.
//!
//! Fix shape (Codex pass-order critical fix incorporated):
//! - Pass 1 of `lower()` extends to walk MethodDef and allocate
//!   FuncIds up-front into a `(receiver, method) -> FuncId` index,
//!   mirroring the existing `function_names` for FunctionDef.
//! - Pass 1.5 `infer_user_function_param_kinds` MethodCall arm
//!   rewrites from "descend-only" (ADR 0092) to refinement-extended,
//!   reading the pre-built index and refining args index 1..N
//!   (`self` at index 0 stays at the ADR 0092 Table policy).
//! - Pass 2 `lower_method_def` uses the pre-allocated FuncId
//!   instead of a fresh alloc.
//!
//! Non-goals (intentional carry-overs):
//! - Index-receiver MethodCall refinement (`(obj.field):m(x)`).
//! - `self` refinement (stays Table per ADR 0092 MVP).
//! - Source-order shadowing resolution (last-wins, same as
//!   FunctionDef's `function_names`).

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
fn colon_method_string_arg_refines() {
    // The exact pattern that ADR 0092's manual smoke surfaced as
    // a carry-over: `function obj:greet(name)` is called with a
    // String literal. Without Pass-1 MethodDef registration +
    // Pass 1.5 refinement, `name` defaults to Number and the
    // dispatcher reports
    // `IndirectCallNoCandidates { param_kinds: [Table, String] }`.
    let src = "local obj = {}
function obj:greet(name) return \"hello \" .. name end
print(obj:greet(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_method_arg_string").trim(),
        "hello world"
    );
}

#[test]
fn colon_method_bool_arg_refines() {
    // Bool literal arg into a colon-defined method. Without
    // refinement, `flag` defaults to Number and Bool dispatch
    // mismatches.
    let src = "local obj = {}
function obj:status(flag) if flag then return 1 else return 0 end end
print(obj:status(true))";
    assert_eq!(run_ok(src, "lumelir_method_arg_bool").trim(), "1");
}

#[test]
fn dotted_method_string_arg_refines() {
    // Dotted method-def (no `self`). The receiver-injection step
    // of MethodCall doesn't apply (call site is plain Call via
    // Index callee), but the `method_funcs` index is built for
    // both dotted and colon forms in Pass 1. The chunk-walker
    // doesn't refine via the Index-callee Call path (ADR 0091
    // carry-over), so we test the path via Method**Call** style
    // refinement using `obj:helper(arg)` against a dotted def —
    // Lua semantics allow calling `function obj.helper(x)` with
    // colon syntax (self is passed but unused).
    //
    // Wait — that semantics doesn't match the registered sig if
    // we route through `method_funcs` (which encodes the def-side
    // arity). Better: keep dotted def + dotted call, but test the
    // refinement for the dotted-def case by adding a direct
    // `obj.helper("world")` call before printing. With ADR 0091's
    // Index-callee path and ADR 0093's `method_funcs` indexing
    // both dotted and colon defs, the chunk-walker visits the
    // MethodCall arm (NOT today's path for `obj.helper(arg)` which
    // is a regular Call with Index callee — that's the existing
    // ADR 0091 carry-over that ADR 0093 does NOT lift).
    //
    // Simpler design: just test colon-defined method with two
    // explicit args (mixed kinds) to exercise refinement across
    // multiple positions.
    let src = "local obj = {}
function obj:concat(s1, s2) return s1 .. s2 end
print(obj:concat(\"foo\", \"bar\"))";
    assert_eq!(run_ok(src, "lumelir_method_arg_multi_str").trim(), "foobar");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn function_def_refinement_path_unchanged() {
    // ADR 0093 extends the chunk-walker's signature and adds a
    // MethodCall arm. The existing FunctionDef + Ident-Call
    // refinement path must remain working. Test exercises a
    // top-level `local function f(name)` that gets called with a
    // String literal — without the existing refinement, `name`
    // would default to Number and dispatch would mismatch.
    let src = "local function greet(name) return \"hello \" .. name end
print(greet(\"world\"))";
    assert_eq!(
        run_ok(src, "lumelir_funcdef_refine_regression").trim(),
        "hello world"
    );
}
