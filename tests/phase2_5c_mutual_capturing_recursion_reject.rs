//! Phase 2.5c-full Commit 3b prep fix (ADR 0083 / ADR 0087 candidate):
//! mutual recursion between two capturing closures cannot be lowered
//! while Function-kind upvalues remain HIR-rejected.
//!
//! When `local function a` and `local function b` both capture
//! upvalues from a shared enclosing scope, calling each other from
//! their respective bodies hits the function_names fallback in
//! `lower_call` (because `lookup_or_capture_upvalue` rejects
//! Function-kind upvalues). With the closure-cell ABI, the cell
//! ptr for the sibling capturing fn cannot be reached from inside
//! the caller's body, so we reject statically rather than
//! miscompile.
//!
//! Self-recursion (`local function fact() ... fact(...) ... end`)
//! is unaffected — the recursion shortcut routes the call through
//! the entry `cell_ptr` block-arg of the capturing fn itself.
//!
//! These tests pin the new `HirError::MutualCapturingRecursion`
//! contract.

#[test]
fn mutual_capturing_recursion_chunk_level_is_rejected() {
    // a and b both capture chunk-level x. a body calls b; b body
    // calls a. Both calls fall through to function_names because
    // Function-kind upvalues are HIR-rejected, but the sibling's
    // cell ptr isn't reachable from either body — reject.
    let chunk = lumelir::parser::parse(
        "local x = 1
local function a() return b() + x end
local function b() return x + 2 end
print(a())",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_err(),
        "mutual capturing recursion (a → b, both capturing) must be HIR-rejected"
    );
}

#[test]
fn self_recursion_capturing_remains_supported() {
    // Self-recursion of a capturing fn must keep working —
    // recursion shortcut (codegen) uses the entry cell_ptr
    // block-arg.
    let chunk = lumelir::parser::parse(
        "local x = 1
local function f(n) if n == 0 then return x end
return f(n - 1) end
print(f(3))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_ok(),
        "self-recursion of a capturing closure must lower"
    );
}

#[test]
fn mutual_non_capturing_recursion_remains_supported() {
    // Mutual recursion between non-capturing fns is unaffected —
    // singleton resolution works for cross-fn calls.
    let chunk = lumelir::parser::parse(
        "local function a(n) if n == 0 then return 0 end
return b(n - 1) end
local function b(n) if n == 0 then return 0 end
return a(n - 1) end
print(a(3))",
    )
    .unwrap();
    assert!(
        lumelir::hir::lower(&chunk).is_ok(),
        "mutual non-capturing recursion must keep lowering"
    );
}
