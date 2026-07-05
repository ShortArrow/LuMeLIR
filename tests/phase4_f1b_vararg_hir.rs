//! ADR 0294 — F1-B: HIR representation of `...`.
//! ADR 0296 — F1-C-step2 upgraded the lowering: `...` now reads
//! `_va_pack[1]` and the synthetic Table param is appended to
//! vararg function signatures. These tests pin the HIR shape.

use lumelir::hir::lower;
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

#[test]
fn hir_lowers_vararg_expr_in_body() {
    let chunk = lower_src("local function f(...) local t = ... end").expect("hir ok");
    assert_eq!(chunk.functions.len(), 1);
    let f = &chunk.functions[0];
    assert!(f.is_vararg);
    // ADR 0296 step2: `_va_pack` is appended as the last param.
    let last = f.params.last().expect("has at least the pack");
    assert_eq!(last.name, "_va_pack");
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
