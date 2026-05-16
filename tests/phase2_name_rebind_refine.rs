//! Phase 2.6+-name-rebind-refine (ADR 0098): Top-level name-rebind
//! refinement via Pass-1.5 alias_map. Closes the ADR 0097
//! future-work for the top-level rebind case:
//!
//! ```ignore
//! local g = a.b.method   -- top-level rebind
//! g(arg)                 -- without this ADR: IndirectCallNoCandidates
//! ```
//!
//! Codex post-0097 review (6 視点) critical fix: use Pass-1.5 pure
//! `alias_map` (chunk-walker builds from `StmtKind::Local/LocalMulti`
//! AST). Don't extend `LocalInfo.func_id` — that mixes pre-pass
//! refinement facts with post-lowering metadata.
//!
//! Non-goals (intentional):
//! - Function-body rebind (`function f() local g = a.b.m; g(x) end`).
//! - Re-assignment alias (`local g; g = a.b.m; g(x)`).
//! - Multi-step alias chains (`local h = a.b.m; local g = h; g(x)`).
//! - Block-scoped shadowing tracking (last-wins per `function_names`).
//! - `local g = some_funcdef` (already handled via ADR 0083).

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
fn single_seg_rebind_string_arg_refines() {
    // `local g = t.helper` rebinds a single-segment method to a
    // local. Without this ADR, `g("world")` refinement skips
    // (g is a local, not in function_names; callee is Ident not
    // Index chain). alias_map["g"] → FuncId(helper) makes refine
    // fire.
    let src = "local t = {}
function t.helper(name) return \"hi \" .. name end
local g = t.helper
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_rebind_single").trim(), "hi world");
}

#[test]
fn multi_seg_rebind_string_arg_refines() {
    // `local g = app.utils.format` rebinds a multi-segment method.
    // extract_index_chain returns `(["app", "utils"], "format")`;
    // method_funcs (chain-keyed per ADR 0097) hits the FuncId.
    let src = "local app = {}
app.utils = {}
function app.utils.format(name) return \"hello \" .. name end
local g = app.utils.format
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_rebind_multi").trim(), "hello world");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn no_rebind_path_unchanged() {
    // The ADR 0097 path (direct Index-callee Call without rebind)
    // must remain working. This verifies the new alias_map
    // construction doesn't break the existing refinement.
    let src = "local app = {}
app.utils = {}
function app.utils.format(name) return \"hi \" .. name end
print(app.utils.format(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_rebind_no_rebind").trim(), "hi world");
}

// --- 1 negative pin (Codex critical: last-wins refinement targeting) ---

#[test]
fn shadowed_rebind_uses_last_def() {
    // Two `local g = ...` rebinds shadow the same name. Codex
    // critical: verify that the dispatcher refines via the LAST
    // alias_map entry (last-wins per insert semantics, matching
    // `function_names` / `method_funcs` shadowing carry-over).
    //
    // First g aliases helper_a (sig (Number,) → Number). Second
    // g aliases helper_b (sig (String,) → String). Call `g("x")`
    // must dispatch to helper_b (the last definition). Without
    // alias_map last-wins, refinement could target helper_a's
    // FuncId, leaving helper_b's `name` at default Number, and
    // String arg would fail dispatch.
    let src = "local t = {}
function t.helper_a(n) return n + 1 end
function t.helper_b(s) return \"hi \" .. s end
local g = t.helper_a
local g = t.helper_b
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_rebind_shadowed").trim(), "hi world");
}
