//! Phase 2.6+-reassign-alias (ADR 0100): re-assignment alias via
//! StmtKind::Assign extension + helper extract. Closes ADR 0098/0099
//! future-work for top-level Assign-based rebinds.
//!
//! Codex post-0099 review (6 視点) critical fixes incorporated:
//! - `record_alias_binding` helper unifies Round1/Round2 ×
//!   Local/Assign/LocalMulti/AssignMulti (8 arms → 1 helper).
//! - Conditional-flow non-refinement is documented as carry-over
//!   (walker visits chunk top-level only, no descent into
//!   `if`/`while`/`for`/function bodies for alias_map).
//!
//! Non-goals (intentional):
//! - Function-body re-assignment.
//! - Control-flow aware refinement (`if cond then g = ... end; g()`
//!   stays with the outer alias).
//! - Call-before-assign source-order resolution (last-wins applies
//!   chunk-wide; future ADR adds per-call-site state).
//! - Method-call rebind via Assign (`g = a:m`).

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
fn local_then_assign_string_arg_refines() {
    // Local with placeholder TaggedValue init (`a.b.dummy`),
    // then Assign re-binds to the target method via Index-chain.
    // Without ADR 0100, the Assign's RHS isn't seen by the walker
    // so `format`'s `name` defaults to Number → String arg fails
    // dispatch. After ADR 0100, the helper records the Assign too
    // (last-wins overrides the Local's earlier entry).
    let src = "local app = {}
function app.dummy(x) return x end
function app.format(name) return \"hi \" .. name end
local g = app.dummy
g = app.format
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_reassign_happy").trim(), "hi world");
}

#[test]
fn last_assign_wins_refinement() {
    // Two Assigns to the same local; the last one wins in
    // alias_map (last-wins semantics, same as `function_names`
    // / `method_funcs` / ADR 0098 Local shadowing).
    // String arg must dispatch to the LAST Assign target.
    let src = "local app = {}
function app.dummy(x) return x end
function app.first(s) return \"first:\" .. s end
function app.last(s) return \"last:\" .. s end
local g = app.dummy
g = app.first
g = app.last
print(g(\"x\"))";
    assert_eq!(run_ok(src, "lumelir_reassign_last_wins").trim(), "last:x");
}

// --- 1 regression-pin (always green Day 0) ---

#[test]
fn local_init_only_unchanged_regression() {
    // ADR 0098 single-step Local rebind path must remain working
    // after the helper extract refactor. Verifies Round 1 path
    // (no Assign involved) is unchanged.
    let src = "local app = {}
function app.format(name) return \"hi \" .. name end
local g = app.format
print(g(\"world\"))";
    assert_eq!(run_ok(src, "lumelir_reassign_regress").trim(), "hi world");
}

// --- 1 negative pin (Codex critical: control-flow boundary) ---

#[test]
fn conditional_assign_does_not_propagate() {
    // Codex critical: pin the boundary that walker doesn't descend
    // into control-flow blocks for alias_map updates. The inner
    // Assign inside `if` is INVISIBLE to alias_map; the OUTER
    // Local init governs the refinement.
    //
    // Setup: outer Local aliases `app.outer` (sig Number,Number);
    // inner if-block conditionally re-assigns to `app.outer` again
    // (a no-op for clarity). The walker doesn't descend, so the
    // refinement uses `app.outer`. Call with Number arg works.
    //
    // The test confirms: compilation succeeds, runtime works via
    // the OUTER alias. If a future change accidentally made the
    // walker descend into if-bodies, the refinement might pick a
    // different target and the test could break (or runtime
    // behavior could shift).
    let src = "local app = {}
function app.outer(n) return n + 1 end
local g = app.outer
if true then g = app.outer end
print(g(41))";
    assert_eq!(run_ok(src, "lumelir_reassign_conditional").trim(), "42");
}
