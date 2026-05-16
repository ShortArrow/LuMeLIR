//! Phase 2.6+-methods (ADR 0092): Method colon syntax desugar over
//! Index-Callee Calls.
//!
//! Builds on ADR 0091 (Index-callee Call normalization). Two
//! syntactic constructs land together:
//!
//! 1. **Call-site** `recv:method(args)` — AST variant
//!    `ExprKind::MethodCall` preserved through parser, desugared at
//!    HIR chokepoint to `Call(callee=Index(recv, Str(method)),
//!    args=[recv, ...args])`. Receiver materialized once (Ident
//!    fast-path; otherwise via the ADR 0091
//!    `materialize_to_synth_local` helper).
//!
//! 2. **Method-def** `function recv:method(...) end` (and dotted
//!    `function recv.field(...) end`) — AST variant `StmtKind::MethodDef`
//!    preserved through parser, desugared at HIR chokepoint to
//!    `IndexAssign(recv, Str(method), FunctionRef)`. For colon form,
//!    `self` (kind `TaggedValue`) is prepended to params and plumbed
//!    via `for_function`'s `external_kinds` seam.
//!
//! Codex post-0091 critical fixes incorporated:
//! - ADR title framing: "Desugar over Index-Callee Calls" (NOT
//!   "sugar-only" — def-side + self policy + receiver-shape check
//!   are infra).
//! - `self` param-kind: `TaggedValue` (most flexible).
//! - HIR-chokepoint desugar (parser preserves AST source shape).
//! - Receiver-shape check explicit: pure recursive walker rejects
//!   any descendant `Call/MethodCall/FunctionExpr/BinOp/UnaryOp` as
//!   `ComplexMethodReceiver`.
//!
//! Non-goals (out of MVP):
//! - Multi-segment method-def (`function a.b.c:m() end`).
//! - Bare top-level `function NAME() end` (globals not supported).
//! - Hetero-return method bodies (LIC-2.6c-tag-locals-fn-indirect-1).
//! - Metatables / `__call`.
//! - `infer_user_function_param_kinds` extension for MethodCall args.

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

// --- 4 happy-path tests ---

#[test]
fn colon_method_def_and_call() {
    // Most basic: define a method via colon syntax, call it via
    // colon syntax. self is threaded as implicit first arg
    // (TaggedValue kind seeded into external_kinds). Body returns
    // a Number derived from the explicit arg.
    let src = "local obj = {}
function obj:add(x) return x + 1 end
print(obj:add(41))";
    assert_eq!(run_ok(src, "lumelir_colon_method_def").trim(), "42");
}

#[test]
fn dotted_method_def_and_call() {
    // Dotted method-def (no implicit self). Equivalent to
    // `obj.helper = function(x) ... end` but using the
    // `function obj.helper(x) end` syntax. Call site uses
    // ADR 0091 Index-callee Call path.
    let src = "local obj = {}
function obj.helper(x) return x * 2 end
print(obj.helper(21))";
    assert_eq!(run_ok(src, "lumelir_dotted_method_def").trim(), "42");
}

#[test]
fn method_with_multiple_args() {
    // Colon method-def with multiple explicit args. Confirms
    // receiver-injection at first arg position correctly handles
    // arity > 2 (self + 3 explicit).
    let src = "local m = {}
function m:sum(a, b, c) return a + b + c end
print(m:sum(10, 20, 30))";
    assert_eq!(run_ok(src, "lumelir_method_multi_args").trim(), "60");
}

#[test]
fn method_def_callable_with_explicit_self() {
    // A colon-defined method is also callable with explicit-self
    // form (`obj.m(obj, ...)`), since the desugar produces a
    // regular IndexAssign-stored function. Pins Lua-spec
    // semantics: the colon is sugar at the call site too.
    let src = "local obj = {}
function obj:add(x) return x + 1 end
local r1 = obj:add(41)
local r2 = obj.add(obj, 100)
print(r1)
print(r2)";
    let stdout = run_ok(src, "lumelir_explicit_self_dual_call");
    let lines: Vec<&str> = stdout.trim().split('\n').collect();
    assert_eq!(lines, vec!["42", "101"]);
}

// --- 1 regression-pin (always green) ---

#[test]
fn existing_index_callee_path_unchanged() {
    // ADR 0091's Index-callee Call path (FunctionExpr stored at
    // a string key, then called via `t.m(args)`) must remain
    // working. ADR 0092 adds NEW paths (MethodCall, MethodDef);
    // existing Index-callee path must not regress.
    let src = "local t = {}
t.compute = function(x) return (x + 1) * 2 end
print(t.compute(3))";
    assert_eq!(run_ok(src, "lumelir_method_regression_pin").trim(), "8");
}

// --- 2 typed-error pins ---

#[test]
fn complex_method_receiver_rejected() {
    // Receiver shape walker rejects any receiver containing
    // Call / MethodCall / FunctionExpr / BinOp / UnaryOp.
    // `(1 + 2):m()` has BinOp receiver — must surface as
    // `HirError::ComplexMethodReceiver` (typed; not the generic
    // UnsupportedCall or a runtime trap).
    let chunk = lumelir::parser::parse(
        "local obj = {}
function obj:m() return 0 end
print((1 + 2):m())",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk).expect_err("expected typed HIR error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ComplexMethodReceiver"),
        "expected ComplexMethodReceiver, got: {msg}"
    );
}

#[test]
fn bare_top_level_function_rejected() {
    // Top-level `function NAME() end` (no receiver dot/colon)
    // requires global function support, which is not implemented.
    // ADR 0092 explicitly rejects this at the parser with
    // `ParseError::UnexpectedToken` at the LParen position
    // (the parser expected `.` or `:` after the receiver name).
    let err = lumelir::parser::parse(
        "function foo() return 0 end
print(foo())",
    )
    .expect_err("expected parse error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("UnexpectedToken") && msg.contains("LParen"),
        "expected UnexpectedToken/LParen, got: {msg}"
    );
}
