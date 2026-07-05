//! ADR 0294 — F1-B: HIR representation of `...`.
//! Codegen still errors — that's F1-C's job. These tests exercise
//! the HIR lowering only.

use lumelir::hir::{HirExprKind, HirStmtKind, lower};
use lumelir::parser::parse;

fn lower_src(src: &str) -> Result<lumelir::hir::HirChunk, lumelir::hir::HirError> {
    let chunk = parse(src).expect("parse ok");
    lower(&chunk)
}

#[test]
fn hir_marks_vararg_function() {
    let chunk = lower_src("local function f(...) end").expect("hir ok");
    // Single function, is_vararg = true.
    assert_eq!(chunk.functions.len(), 1);
    assert!(chunk.functions[0].is_vararg);
}

#[test]
fn hir_marks_non_vararg_function() {
    let chunk = lower_src("local function f(a, b) end").expect("hir ok");
    assert_eq!(chunk.functions.len(), 1);
    assert!(!chunk.functions[0].is_vararg);
}

fn body_contains_vararg(stmts: &[lumelir::hir::HirStmt]) -> bool {
    for s in stmts {
        match &s.kind {
            HirStmtKind::LocalInit { value, .. } | HirStmtKind::Assign { value, .. }
                if matches!(value.kind, HirExprKind::Vararg) =>
            {
                return true;
            }
            HirStmtKind::If {
                then_body,
                elifs,
                else_body,
                ..
            } => {
                if body_contains_vararg(then_body) {
                    return true;
                }
                for (_, elif_body) in elifs {
                    if body_contains_vararg(elif_body) {
                        return true;
                    }
                }
                if let Some(eb) = else_body
                    && body_contains_vararg(eb)
                {
                    return true;
                }
            }
            HirStmtKind::While { body, .. } if body_contains_vararg(body) => return true,
            HirStmtKind::Block { stmts } if body_contains_vararg(stmts) => return true,
            _ => {}
        }
    }
    false
}

#[test]
fn hir_lowers_vararg_expr_in_body() {
    let chunk = lower_src("local function f(...) local t = ... end").expect("hir ok");
    assert_eq!(chunk.functions.len(), 1);
    let f = &chunk.functions[0];
    assert!(f.is_vararg);
    assert!(
        body_contains_vararg(&f.body),
        "expected HirExprKind::Vararg somewhere in body"
    );
}

#[test]
fn hir_rejects_vararg_outside_vararg_function() {
    // `local function f() local t = ... end` — non-vararg fn using ...
    // should error at HIR.
    let src = "local function f() local t = ... end";
    let r = lower_src(src);
    assert!(r.is_err(), "expected HIR error: {r:?}");
}

#[test]
fn hir_rejects_vararg_at_chunk_level() {
    // `...` at chunk-level is spec-legal (Lua treats chunks as
    // implicit vararg), but our impl doesn't wire a chunk-level
    // vararg yet — HIR errors.
    let src = "local t = ...";
    let r = lower_src(src);
    assert!(r.is_err(), "expected HIR error at chunk level: {r:?}");
}
