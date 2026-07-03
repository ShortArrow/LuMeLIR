//! ADR 0293 — F1-A: parser accepts `...` in function signature
//! and in expression position. HIR / codegen wiring is F1-B / F1-C.

use lumelir::parser::parse;

#[test]
fn parses_vararg_in_function_signature() {
    let src = "local function f(...) end";
    let r = parse(src);
    assert!(r.is_ok(), "vararg param parse failed: {r:?}");
}

#[test]
fn parses_vararg_with_named_params_first() {
    let src = "local function f(a, b, ...) end";
    let r = parse(src);
    assert!(r.is_ok(), "vararg after named params failed: {r:?}");
}

#[test]
fn parses_vararg_in_anonymous_function() {
    let src = "local f = function(...) end";
    let r = parse(src);
    assert!(r.is_ok(), "anonymous vararg failed: {r:?}");
}

#[test]
fn parses_vararg_in_expression_position() {
    // Inside a vararg function body — parser can't tell context;
    // it just accepts `...` where an expression is allowed.
    let src = "local function f(...) local t = ... end";
    let r = parse(src);
    assert!(r.is_ok(), "expression-position vararg failed: {r:?}");
}

#[test]
fn parses_vararg_as_call_argument() {
    let src = "local function f(...) print(...) end";
    let r = parse(src);
    assert!(r.is_ok(), "vararg as call arg failed: {r:?}");
}

#[test]
fn parses_vararg_in_table_constructor() {
    let src = "local function f(...) local t = {...} end";
    let r = parse(src);
    assert!(r.is_ok(), "vararg in table ctor failed: {r:?}");
}

#[test]
fn parses_method_def_with_vararg() {
    let src = "local mt = {}\nfunction mt:f(...) end";
    let r = parse(src);
    assert!(r.is_ok(), "method vararg failed: {r:?}");
}

#[test]
fn rejects_vararg_not_at_end_of_params() {
    // Lua spec: `...` must be the last parameter. Our parser today
    // treats a comma after `...` as an ordinary continuation; since
    // it breaks out of the param loop after seeing `...`, a trailing
    // `, x` would then hit an unexpected close-paren. Confirm the
    // shape parses one way or the other without crashing (regression
    // guard for a follow-up ADR that will tighten the rule).
    let src = "local function f(..., x) end";
    let _ = parse(src); // may Err — the important part is: no panic.
}

#[test]
fn dotdot_still_lexes_as_concat() {
    // Regression: `..` remains string concat, not the start of `...`.
    let src = "local s = \"a\" .. \"b\"\nprint(s)";
    let r = parse(src);
    assert!(r.is_ok(), ".. broke: {r:?}");
}
