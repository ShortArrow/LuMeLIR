# 0043. Phase 2.5c.2: Bool / Nil / String Upvalue Captures

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.5c-min (ADR 0037) shipped capture-by-value closures
restricted to Number upvalues. The restriction was a TDD
guardrail, not a structural one — codegen's
`emit_alloca_slot_for_kind` and `param_mlir_type` helpers
already cover Bool (i1), Nil (i1), and String (`!llvm.ptr`),
because every other slot-allocating code path needs them.

What blocked non-Number captures was a single `if outer_kind
!= ValueKind::Number` branch in HIR's
`lookup_or_capture_upvalue`. With it gone, the existing
codegen path "just works" — the upvalue list, the call-site
arg extension, the function-signature widening, and the
inner-slot store at function entry are all kind-generic
already.

The remaining hold-out is **Function-kind upvalues**, which
need a different path entirely (see Out of Scope).

## Decision

### Replace the equality check with an inverted reject

```rust
// Before (2.5c-min):
if outer_kind != ValueKind::Number {
    return Err(HirError::TypeMismatch { ... });
}

// After (2.5c.2):
if matches!(outer_kind, ValueKind::Function(_)) {
    return Err(HirError::TypeMismatch { ... });
}
```

Number, Bool, Nil, and String captures all flow through
unchanged. The `inner_local_id` declaration uses
`outer_kind` directly, so the inner local carries the right
kind for body-side `HirExprKind::Local` resolution; the
codegen alloca + store + load loop never sees a special case.

### Why Function still rejects

Function-kind locals don't follow the alloca-backed slot
pattern. The current code at `emit_function`:

```rust
let slots: Vec<Value<'c, '_>> = hir_fn
    .locals
    .iter()
    .enumerate()
    .map(|(i, info)| match info.kind {
        ValueKind::Function(_) if i < hir_fn.params.len() => block.argument(i).unwrap().into(),
        _ => emit_alloca_slot_for_kind(...),
    })
    .collect();
```

Function-kind *params* take the block argument as the slot
itself; non-param Function-kind locals get an `i1`
placeholder slot whose value is reproduced via
`func.constant` + `LocalInfo.func_id` at every use site. A
captured Function upvalue would be neither: it'd need its
inner local to either (a) be declared after the params (so
the block-argument-as-slot trick doesn't apply) or (b) have
`func_id` threaded from the outer's `LocalInfo.func_id`.

Both routes are real work — and the right place to do that
work is alongside the upcoming
[Phase 2.5c-full] effort that introduces (fn_ptr, env_ptr)
closure values, not as a one-off lift.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | One predicate flip in `lookup_or_capture_upvalue`; updated diagnostic message |
| Codegen  | None                                                |

The changed-line count is genuinely small (~10 lines net,
mostly the diagnostic text). The structural work was done in
2.5c-min.

## TDD Process

1. **Red.** 8 e2e tests added covering Bool capture (basic +
   branch driver), String capture (basic + concat), Nil
   capture, mixed Number+String, nested-function Bool
   capture, and the Function-rejection boundary. Seven
   failed; one (Function rejection) already passed because
   the old broad reject still fired.
2. **Green.** One predicate flip. All 8 e2e tests pass.
3. **Refactor.** Two pre-existing tests reframed:
   - HIR unit `lower_closure_capturing_bool_is_static_error`
     → renamed to
     `lower_closure_capturing_function_is_static_error`,
     with a sibling `…_now_succeeds_after_2_5c2` that pins
     the new behaviour at the HIR level.
   - E2E `closure_capturing_bool_is_static_error` →
     renamed to `closure_capturing_function_is_static_error`,
     using a Function source so the boundary stays useful.

## Alternatives Considered

- **Allow Function-kind captures in this phase.** Worth
  doing eventually, but the codegen change is non-local
  (slot allocation, body emit, and the call-site arg-extension
  helper all need to learn the function-pointer carrier).
  Not the same kind of "one predicate flip" delta this phase
  is. Defer to 2.5c-full.
- **Drop the diagnostic entirely and emit a lower-level
  codegen error**. Static rejection in HIR is the right
  layer — the user gets the diagnostic before MLIR sees
  anything malformed. Keep.
- **Allow Function captures via lambda lifting that snapshots
  the FuncId at capture time** (treating it as a constant the
  inner can rebuild). Unsound for closures: if the outer slot
  is later reassigned to a different function, the inner
  would still see the old FuncId. Defer.

## Consequences

- HIR predicate flip; ~10-line net diff.
- 8 new e2e tests; 1 new HIR unit test; 2 pre-existing tests
  reframed. Total green at 596.
- ADR 0037's "Out of Scope: Non-Number captures
  (Bool/Nil/String/Function)" item is partially retired —
  Bool, Nil, and String land here; Function persists.

## Out of Scope

- **Function-kind captures.** Pending the (fn_ptr, env_ptr)
  closure-value rework.
- **Closure escape with non-Number captures.** Same as 2.5c-min
  — the upvalue is the outer slot, and the closure can't
  outlive that slot's scope.
- **Snapshot semantics for non-Number captures.** Same
  live-binding rule applies as for Number — the inner reads
  the outer slot at every call.
