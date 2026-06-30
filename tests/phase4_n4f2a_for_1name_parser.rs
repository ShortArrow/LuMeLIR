//! ADR 0285 — N4-F-2a: `for x in iter do ... end` 1-name parser
//! shape. The parser now accepts the 1-name form by desugaring
//! to the existing 2-name ForGeneric with a synthetic discard
//! binding and nil state/ctl. End-to-end execution still
//! requires a closure-returning iter source (N4-F-2b/2c).

use lumelir::parser::parse;

#[test]
fn for_1name_parses_against_local_iter() {
    // We only test the parse stage — execution requires a closure-
    // returning iter that produces (TaggedValue, TaggedValue),
    // which N4-F-2b/c are still designing.
    let src = "local iter = nil\nfor x in iter do print(x) end";
    let r = parse(src);
    assert!(r.is_ok(), "1-name for-in parse failed: {r:?}");
}

#[test]
fn for_1name_parses_with_function_call_iter() {
    let src = "local function f() return nil end\nfor x in f() do print(x) end";
    let r = parse(src);
    assert!(r.is_ok(), "1-name for-in with call iter failed: {r:?}");
}

#[test]
fn for_1name_parses_with_complex_iter() {
    // Just exercise parser acceptance with a non-trivial iter expr.
    let src = "local t = {}\nfor x in t.iter do print(x) end";
    let r = parse(src);
    assert!(r.is_ok(), "1-name for-in with .iter failed: {r:?}");
}
