//! Phase 2.6+-multi-step-alias (ADR 0099): top-level multi-step
//! alias chain resolution via fixed-point alias_map. Closes the
//! ADR 0098 future-work for `local h = a.b.m; local g = h; g(x)`
//! Ident→Ident rebinding.
//!
//! Codex post-0098 review (6 視点) critical fix: incorporate the
//! fixed-point into ADR 0098's `alias_map` build phase (NOT a
//! separate Call-side helper). Insert-only monotonic for
//! guaranteed termination.
//!
//! Non-goals (intentional):
//! - Function-body multi-step alias (chunk-walker top-level only).
//! - Re-assignment alias (`local g; g = a.b.m; g(x)`).
//! - Block-scoped scope tracking.
//! - Method-call rebind (`local g = a:m; g(x)`).

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

// --- 2 happy-path tests (Red Day 0, Green after Step 1) ---

#[test]
fn two_step_alias_chain_refines() {
    // The exact pattern ADR 0098 future-work flagged:
    // `local h = a.b.m` populates alias_map["h"] (Round 1
    // Index-chain). `local g = h` is a bare Ident value; today's
    // ADR 0098 build doesn't propagate. ADR 0099 fixed-point
    // Round 2 fills alias_map["g"] = alias_map["h"]. Call refines.
    let src = "local app = {}
app.utils = {}
function app.utils.format(name) return \"hi \" .. name end
local h = app.utils.format
local g = h
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_alias_2step").trim(), "hi world");
}

#[test]
fn three_step_alias_chain_refines() {
    // 3-step boundary case to pin fixed-point convergence
    // correctness when chain depth > 2. `local i = a.b.m;
    // local h = i; local g = h; g(x)`. Round 1 populates `i`;
    // Round 2 propagates to `h`; Round 3 propagates to `g`.
    let src = "local t = {}
function t.helper(name) return \"hello \" .. name end
local i = t.helper
local h = i
local g = h
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_alias_3step").trim(), "hello world");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn single_step_alias_unchanged() {
    // ADR 0098 single-step rebind path must remain working after
    // the Round 2+ fixed-point extension. Codex critical: this
    // regression-pin SEPARATELY verifies single-step path is not
    // perturbed.
    let src = "local app = {}
app.utils = {}
function app.utils.format(name) return \"hi \" .. name end
local g = app.utils.format
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_alias_single").trim(), "hi world");
}
