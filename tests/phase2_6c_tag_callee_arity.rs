//! Phase 2.6c-tag-callee-arity (ADR 0075): tagged-callee arity
//! hardening — indirect calls through TaggedValue locals are
//! HIR-rejected. Closes LIC-2.6c-tag-callee-arity-1 by removing
//! the unsafe path entirely. ADR 0072's `local g = t[k]; g()`
//! pattern is rolled back; users must use direct calls or
//! static dispatch instead.
//!
//! The dropped feature was always UB-prone: `args.len()`
//! reconstruction at the call site assumed the slot's payload
//! function had matching arity. ADR 0074 only patched the
//! TaggedValue-return case; this ADR covers all heterogeneous
//! sources at the source.

#[test]
fn array_indexed_function_call_rejected_at_hir() {
    // Single-arity table — still rejected because HIR can't
    // statically prove the table holds a single arity (would
    // require flow analysis ADR 0075 explicitly defers).
    let chunk = lumelir::parser::parse(
        "local function f() return 42 end
local t = {f}
local g = t[1]
print(g())",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "indirect call through TaggedValue local must be HIR-rejected"
    );
}

#[test]
fn hash_indexed_function_call_rejected_at_hir() {
    let chunk = lumelir::parser::parse(
        "local function f() return 7 end
local t = {}
t.f = f
local g = t.f
print(g())",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn heterogeneous_arity_table_call_rejected_at_hir() {
    // The LIC-2.6c-tag-callee-arity-1 hazard case: two
    // functions with different arities in the same table.
    // Pre-ADR-0075 this compiled and reconstructed
    // `(f64, f64) -> f64` from args.len(); calling f1 with
    // 2 args was UB. Now HIR-rejected.
    let chunk = lumelir::parser::parse(
        "local function f1(x) return x end
local function f2(x, y) return x + y end
local t = {f1, f2}
local g = t[1]
print(g(1, 2))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "heterogeneous-arity TaggedValue indirect call must be HIR-rejected"
    );
}

#[test]
fn direct_function_call_via_known_local_still_works() {
    // Regression: a Function-kind local with a known FuncId
    // (alias of a top-level `local function`) remains
    // callable. This is the static-arity path ADR 0075
    // preserves.
    use std::process::Command;
    let output = std::env::temp_dir().join("lumelir_arity_direct");
    let chunk = lumelir::parser::parse(
        "local function f() return 42 end
local g = f
print(g())",
    )
    .unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(
        result.status.success(),
        "direct call regression: {result:?}"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn function_parameter_indirect_call_still_works() {
    // Regression: function-parameter Function locals
    // (Phase 2.5b.2 / ADR 0018) carry a statically-known
    // arity inferred via body scan; their `Callee::Indirect`
    // path is safe and remains supported.
    use std::process::Command;
    let output = std::env::temp_dir().join("lumelir_arity_param");
    let chunk = lumelir::parser::parse(
        "local function apply(g, x) return g(x) end
local function f(x) return x * 2 end
print(apply(f, 21))",
    )
    .unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output).output().unwrap();
    let _ = std::fs::remove_file(&output);
    assert!(
        result.status.success(),
        "param indirect regression: {result:?}"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert_eq!(stdout.trim(), "42");
}
