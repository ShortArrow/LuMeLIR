# 0044. Phase 2.5c.3: Static Rejection of Closure Escape

- **Status:** **Superseded by ADR 0083 Commit 3c** (2026-05-10)
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Supersede note

ADR 0083 Commit 3c (full-closures cell-ptr-first ABI) replaced
this static rejection with a runtime-sound implementation:
heap-allocated closure cells + heap-allocated upvalue boxes
survive any frame teardown, so a closure value can pass through
`Callee::Indirect`, return values, table slots, and arguments
without losing access to its captured bindings. The variant
`HirError::ClosureEscapes` and the helper `closure_with_upvalues`
have been removed; `LIC-2.6c-tag-hetero-closure-escape-1` is
resolved.

The remaining backstop is `HirError::MutualCapturingRecursion`
(post-pass walking call sites), which catches the narrow case
two capturing siblings calling each other from their respective
bodies — codegen still has no path to reach a sibling's per-
instance cell from inside the caller's body. That backstop is
expected to retire under ADR 0088 (Function-kind upvalue support
or TaggedValue arith coerce; ADR 0087 was reassigned to hash-key
runtime validity policy on 2026-05-10).

## Context

ADR 0037 (Phase 2.5c-min) noted a sharp limitation in the
capture-by-value scheme: closures with upvalues can only be
reached via direct calls. The call-site code that threads
upvalues as extra arguments lives only on the `Callee::User`
paths (`lower_call`'s local-Function-kind branch and its
function_names branch). Any path that routes a closure value
through `Callee::Indirect` — passing it as a function-typed
argument, returning it from a function — silently drops the
upvalue list.

The user-visible result was a cryptic MLIR-level diagnostic:

```
error: 'func.constant' op reference to function with mismatched type
lumelir: codegen error: MLIR module verification failed
```

The signature mismatch is a *consequence* of upvalue dropping
(the closure's MLIR signature is `(params, upvalues)` but the
indirect call site's expected type is `(params)`); the user
has no way to map that error back to "I cannot pass a closure
as an argument."

This phase converts that into a clear HIR-level diagnostic.

## Decision

### `HirError::ClosureEscapes`

A new error variant carries the position string ("call
argument" / "return value") and the offset:

```rust
#[error(
    "closure with upvalues cannot escape via {position} — direct call only (offset {offset})"
)]
ClosureEscapes { position: String, offset: usize },
```

### `LowerCtx::closure_with_upvalues` helper

A pure inspector returning `Option<FuncId>`:

```rust
fn closure_with_upvalues(&self, expr: &HirExpr) -> Option<FuncId> {
    let fid = match &expr.kind {
        HirExprKind::FunctionRef(fid) => *fid,
        HirExprKind::Local(LocalId(idx)) => self.locals[*idx].func_id?,
        _ => return None,
    };
    if self.functions[fid.0].upvalues.is_empty() {
        None
    } else {
        Some(fid)
    }
}
```

Hits any HIR expression that evaluates to a Function value
with non-empty `upvalues`. The `Local` arm catches alias
bindings (`local g = f` propagates `func_id` so `g` here is a
hit when `f` was). The `FunctionRef` arm catches inline
anonymous closures (`apply(function() ... end, x)`).

### Two call sites for the check

1. **`lower_call` — both Callee::User branches** (the local
   path and the function_names path): after kind-checking each
   lowered arg, also reject if it is a closure-with-upvalues.
2. **`lower_return_with_values`**: before any kind processing,
   walk each lowered return value and reject if any is a
   closure-with-upvalues.

Direct-call sites are unaffected: `lower_call`'s Callee::User
branches only check the *args*, not the callee. So `f(x)`
where `f` is a closure-with-upvalues still threads upvalues
correctly — the `f` reaches the `Callee::User` resolution
path, not through arg lowering.

### CA invariants preserved

| Layer    | Change                                                  |
|----------|---------------------------------------------------------|
| Lexer    | None                                                    |
| Parser   | None                                                    |
| AST      | None                                                    |
| HIR      | New `ClosureEscapes` variant; one pure helper (`closure_with_upvalues`); three call-site insertions |
| Codegen  | None — the reject fires before any expression with a malformed shape reaches MLIR |

## TDD Process

1. **Red.** 8 e2e tests covering: pass closure as arg
   (basic + via alias + inline anonymous form); return
   closure; direct-call regression; alias-then-direct-call
   regression; pass-no-upvalue-function regression; return-
   no-upvalue-function regression. Three new "should error"
   tests failed (the rejection didn't yet exist); the others
   already passed (some via codegen-verifier rejection,
   confirming the bug was real but fired later in the
   pipeline).
2. **Green.** `ClosureEscapes` variant + `closure_with_upvalues`
   helper + three call-site insertions. The codegen-verifier
   test cases now fail at HIR with the clear diagnostic.
3. **Refactor.** No duplication emerged — the three
   insertions all match shape `if self.closure_with_upvalues(...)
   .is_some() { return Err(...) }` with different position
   strings. Extracting a fourth helper would trade three
   lines for a one-liner; rule of three not met.

## Alternatives Considered

- **Keep the silent-drop and document it.** Discarded — the
  failure mode (cryptic MLIR error) was the worst kind: the
  compiler refuses to produce a binary, but the error
  message points at internals. Static rejection at the
  source-level boundary gives the best handover.
- **Allow the escape and fix codegen** to either thread
  upvalues through Indirect (would need closure values
  to carry an env pointer) or copy upvalues into a
  dedicated tagged union. Both are real Phase 2.5c-full
  work — they belong with the (fn_ptr, env_ptr) closure
  ABI rework, not as a one-off.
- **Reject Function-kind values broadly** (any non-direct
  use of any Function value, with or without upvalues).
  Would also block legitimate first-class function patterns
  from Phase 2.5b.2/2.5b.3. Rejected — the predicate must
  match exactly the values that lose information.

## Consequences

- HIR adds one error variant + ~30 lines (helper + three
  insertions).
- 8 new e2e tests covering both the rejection and the
  preserved-direct-call paths; total green at 604.
- ADR 0037's "Out of Scope: Static `ClosureEscapes`
  rejection for closures with upvalues being passed to
  `apply(f, ...)`-style sinks" item retires.

## Out of Scope

Closure escape is now an *error*, not a feature. Allowing it
remains future work:

- **(fn_ptr, env_ptr) closure values** — Phase 2.5c-full.
- **Heap-allocated upvalue cells** — Phase 2.5c-full.
- **Function-kind upvalues** — pending the closure-value rework
  (still rejected per ADR 0043).
- **Mutually-capturing nested functions** (`a` captures `b`,
  `b` captures `a`).
