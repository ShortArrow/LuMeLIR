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
    // a and b both capture chunk-level x. a body calls b; the
    // call falls through to function_names because Function-kind
    // upvalues are HIR-rejected, but b's cell ptr isn't reachable
    // from inside a's body — reject.
    let chunk = lumelir::parser::parse(
        "local x = 1
local function a() return b() + x end
local function b() return x + 2 end
print(a())",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk)
        .expect_err("mutual capturing recursion (a → b, both capturing) must be HIR-rejected");
    // Phase 2.5c-full Commit 3b prep fix v2 (Codex P2): the
    // diagnostic surfaces the Lua-level name. The exact callee
    // observed in this snippet is `b`.
    let msg = err.to_string();
    assert!(
        msg.contains("'b'"),
        "diagnostic should name the Lua-level callee 'b', got:\n{msg}",
    );
}

#[test]
fn mutual_capturing_recursion_bidirectional_chunk_is_rejected() {
    // Both a and b body call the sibling — the post-pass walks
    // all bodies, so either direction (whichever the post-pass
    // lands on first) must be reported. This test pins that
    // bidirectional captures don't slip through.
    let chunk = lumelir::parser::parse(
        "local x = 1
local function a() return b() + x end
local function b() return a() + x end
print(a())",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk)
        .expect_err("bidirectional mutual capturing recursion must be HIR-rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("'a'") || msg.contains("'b'"),
        "diagnostic should name 'a' or 'b' as the rejected sibling, got:\n{msg}",
    );
}

#[test]
fn mutual_capturing_recursion_nested_inside_outer_is_rejected() {
    // Two siblings declared in outer's body, both capturing
    // outer's `x`. inner_a calls inner_b — fall through to
    // function_names because inner_b is a synthetic body-local
    // of outer (not in inner_a's body's scope) and the
    // Function-kind upvalue is rejected. Same reject path as
    // chunk-level mutual capturing, just shifted one scope deeper.
    let chunk = lumelir::parser::parse(
        "local function outer(x)
  local function inner_a() return inner_b() + x end
  local function inner_b() return x + 2 end
  return inner_a()
end
print(outer(5))",
    )
    .unwrap();
    let err = lumelir::hir::lower(&chunk)
        .expect_err("nested mutual capturing recursion must be HIR-rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("'inner_b'") || msg.contains("'inner_a'"),
        "diagnostic should name an inner sibling, got:\n{msg}",
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
