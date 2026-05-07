//! Phase 2.5c-full Commit 2a-fix (ADR 0083 / ADR 0075 amend):
//! reject parameter-routed indirect calls whose source function
//! returns a non-Number kind.
//!
//! Background — Codex review of commit 551d51c (ADR 0083 Commit 2a)
//! found that `Callee::Indirect`'s codegen hardcodes `Some(types.f64)`
//! as the call result type. Before Commit 2a, the MLIR verifier
//! shielded that mismatch via the `!func.func<...>` typed value;
//! after Commit 2a's `!llvm.ptr` erasure the verifier can no longer
//! catch it, so a `Bool`/`Nil`/`String`/`Table`-returning user fn
//! routed through a `Function`-kind parameter would silently
//! miscompile as `f64`.
//!
//! ADR 0075's reject set already excludes TaggedValue-routed indirect
//! calls; this amend extends the same backstop to parameter-routed
//! ones until ADR 0087 lifts the restriction by routing through
//! `Callee::IndirectDispatch`.
//!
//! These tests pin the HIR-level reject: every non-Number return
//! kind on a function value flowing into a `Function`-kind parameter
//! must produce a `lower` error. The Number-return regression test
//! confirms the existing happy path still lowers cleanly.

#[test]
fn bool_return_routed_through_function_param_is_rejected() {
    let chunk = lumelir::parser::parse(
        "local function pred(x) return x > 0 end
local function apply(g, x) return g(x) end
print(apply(pred, 1))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "Bool-returning fn routed through Function-kind param must be HIR-rejected"
    );
}

#[test]
fn nil_return_routed_through_function_param_is_rejected() {
    let chunk = lumelir::parser::parse(
        "local function n() return nil end
local function apply(g) return g() end
print(apply(n))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "Nil-returning fn routed through Function-kind param must be HIR-rejected"
    );
}

#[test]
fn string_return_routed_through_function_param_is_rejected() {
    let chunk = lumelir::parser::parse(
        "local function s() return \"hi\" end
local function apply(g) return g() end
print(apply(s))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "String-returning fn routed through Function-kind param must be HIR-rejected"
    );
}

#[test]
fn multi_return_routed_through_function_param_is_rejected() {
    // ret_kinds = [Number, Number] is also non-[Number] (length > 1)
    // — Callee::Indirect's f64 single-result assumption breaks.
    let chunk = lumelir::parser::parse(
        "local function pair() return 1, 2 end
local function apply(g) return g() end
print(apply(pair))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "multi-return fn routed through Function-kind param must be HIR-rejected"
    );
}

#[test]
fn number_return_routed_through_function_param_still_works() {
    // Regression: the existing happy path that motivated `Callee::Indirect`
    // (Phase 2.5b.2 first-class function args) must keep lowering.
    let chunk = lumelir::parser::parse(
        "local function double(x) return x * 2 end
local function apply(g, x) return g(x) end
print(apply(double, 5))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_ok(),
        "Number-returning fn through Function-kind param must still lower"
    );
}
