# 0179. Non-Local TaggedValue Source — Uniform HIR Synth-Local Materialisation

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-07
- **Deciders:** ShortArrow

## Context

[ADRs 0084 / 0174 / 0175 / 0176 / 0177](#references) all share an identical scope-ceiling clause: "TaggedValue source must be `Local(LocalId)` only." Codegen sites that consume a TaggedValue key (or, in ADR 0176, value) match on `HirExprKind::Local(LocalId(idx))` and either `unreachable!()` or `return Err(CodegenError::UnsupportedExpr)` otherwise.

This blocks idiomatic patterns the language permits:

```lua
local t1, t2 = {}, {}
-- ...
local x = t1[t2[k]]            -- IndexTagged key: t2[k] is an IndexTagged expr, not a Local
print(rawget(t1, f()))          -- rawget arg[1]: f() returns TaggedValue, not a Local
rawset(t1, h(), g())            -- rawset arg[1] AND arg[2]: both non-Local TaggedValue
```

Per the [sweep 0166-0177 retrospective §Horizontal duplication](../notes/sweep-0166-0177-retrospective.md), opening these one site at a time would re-introduce the ad-hoc duplication that ADR 0178 just collapsed. The correct fix is **structural**: HIR pre-materialises any non-Local TaggedValue source into a synth local before lowering reaches the dispatcher sites. Codegen then stays oblivious — every source is already `Local`.

The mechanism is reuse: `materialize_to_synth_local` already exists (introduced by ADR 0091 / renamed in ADR 0092), used for callee-position pre-binding. The same machinery extends to source positions.

## Scope (literal)

- ✅ HIR materialisation at **4 source positions**:
  1. `IndexTagged { target, key }` — when `key` has `infer_kind == TaggedValue` and `key.kind != Local(_)`.
  2. `Callee::Builtin(RawGet)` arg[1] — same condition.
  3. `Callee::Builtin(RawSet)` arg[1] (key) — same condition.
  4. `Callee::Builtin(RawSet)` arg[2] (value) — when key is `Local(TaggedValue)` AND value has TaggedValue kind AND `value.kind != Local(_)`.
- ✅ Materialisation reuses `materialize_to_synth_local` (allocates `__synth_NNN` local, inserts `LocalInit` into `pending_pre_stmts`, returns `Local(synth_id)`).
- ✅ Codegen `_ => return Err(...)` / `_ => unreachable!()` arms guarding the Local check at the 4 sites become unreachable in practice; tightened or kept as defensive `unreachable!()`.
- ❌ Non-TaggedValue source kinds (Number, String, Bool, Function, Table) — already handled via `emit_expr` at every site; no change.
- ❌ `IndexAssign` value side (`t[k] = non_local_tagged_expr`) — separate code path (`emit_index_assign`), separate ADR if needed. The deferral row stays open for that one position.
- ❌ Function-form metamethod call ABI (ADRs 0141/0142+) — orthogonal.

## Decision

### HIR side

#### New helper: `materialize_tagged_source_if_needed`

```rust
fn materialize_tagged_source_if_needed(
    &mut self,
    expr: HirExpr,
    synth_span: Span,
) -> Result<HirExpr, HirError> {
    let kind = infer_kind(&expr, &self.locals, &self.functions);
    if !matches!(kind, ValueKind::TaggedValue) {
        return Ok(expr);
    }
    if matches!(expr.kind, HirExprKind::Local(_)) {
        return Ok(expr);
    }
    // Lower the expression into a synth Local of TaggedValue kind,
    // emit LocalInit into pending_pre_stmts, and replace the
    // original expression with Local(synth_id).
    let seq = self.callee_seq;
    self.callee_seq += 1;
    let synth_name = format!("__tagged_src_{seq}");
    let synth_id = self.declare_local(synth_name, ValueKind::TaggedValue);
    let widened = widen_index_for_local_init(expr);
    self.pending_pre_stmts.push(HirStmt {
        kind: HirStmtKind::LocalInit { id: synth_id, value: widened },
        span: synth_span,
    });
    Ok(HirExpr {
        kind: HirExprKind::Local(synth_id),
        span: synth_span,
    })
}
```

Note: takes `HirExpr` (already-lowered) rather than `&Expr` because the call sites have already lowered through `lower_expr`. Reuses `callee_seq` (single global synth counter — ADRs 0091/0092 precedent).

#### Call sites

| Site | Change |
|---|---|
| `Index` → `IndexTagged` lowering (at `widen_index_for_local_init` / construction site) | Apply helper to the `key` field BEFORE building IndexTagged. |
| `lower_builtin_call` arg loop, idx==1, builtin=RawGet/RawSet | After lowering arg, if kind is TaggedValue and not Local, apply helper. |
| `lower_builtin_call` arg idx==2, builtin=RawSet, key is Local(TaggedValue) | Same; helper handles the kind check. |

#### HIR validation relaxation

In `lower_builtin_call`:
- Drop the `&& matches!(arg.kind, HirExprKind::Local(_))` clause from `rawget_tagged_local_ok` and `rawset_tagged_local_ok` (rename to `_tagged_ok`). Materialisation guarantees Local by the time emit sees the arg.
- Tighten the post-arg-2-rejection (ADR 0175) — if key is `Local(TaggedValue)` and value is TaggedValue, materialisation has already converted any non-Local value to Local. The check becomes vacuous; remove it.

### Codegen side

At the 4 dispatcher sites, the `_ => return Err(CodegenError::UnsupportedExpr(...))` and `_ => unreachable!(...)` arms become reachable only via a HIR bug — keep as `unreachable!()` with updated message ("HIR ADR 0179 materialisation guarantees Local source") to surface contract violation if regressed.

No structural codegen change; site code keeps using `slots[idx]` directly.

### Tests

`tests/phase2_6plus_non_local_tagged_source.rs` (NEW, ~6 e2e):

1. `t1[t2[k]]` — IndexTagged key is itself IndexTagged.
2. `rawget(t, fn_returning_tagged())` — Call-return source.
3. `rawset(t, fn_returning_tagged(), v)` — Call-return key.
4. `rawset(t, k_local, fn_returning_tagged())` — Call-return value with Local TaggedValue key.
5. `local m = t1[t2[k]]; rawget(t1, m)` — verify synth local is materialised once, not twice (sanity).
6. `__index` chain: `t1[t2[k]]` where t1 has `__index` referring to a 2nd table — confirms materialisation doesn't bypass the metatable chain.

## Alternatives considered

- **Codegen-side materialisation** (emit_expr returns slot ptr for any TaggedValue expr; sites call emit_expr unconditionally). Rejected — IndexTagged's `emit_expr` arm is currently `unreachable!()`; reaching it would require duplicating the IndexTagged dispatch logic in expr position, exactly the kind of horizontal duplication ADR 0178 just removed.
- **Open one source position at a time** (Call-return only, then IndexTagged later, …). Rejected — explicit ad-hoc anti-pattern per Codex #1 and the retrospective's "next moves" warning.
- **Mark all non-Local TaggedValue source as HIR error** (no materialisation; force user rewrite). Rejected — Lua spec permits these patterns; the compiler should respect that.
- **Materialise unconditionally** (every TaggedValue arg → synth local, even Local). Rejected — pointless `LocalInit __synth = Local(other)` reads churn pending_pre_stmts and obscure IR.

## Consequences

**Positive**
- 5 ADRs' "Local source only" deferral notes are resolved at the source-position level in one move.
- Future ADRs touching TaggedValue source positions don't re-encounter this restriction.
- Synth-local mechanism is reused (no new HIR machinery).
- Codegen sites stay unchanged structurally; just the `unreachable!()` messages update.

**Negative**
- IR grows by one `LocalInit` per non-Local source — minor LIR-level cost; LLVM constant-folds trivial chains.
- Synth-local name collisions impossible by construction (`callee_seq` is monotone).
- IndexAssign value side stays restricted; documented as deferral.

**Locked in until superseded**
- The 4 source-position list. Adding a 5th position (e.g. IndexAssign value side) requires an explicit follow-up ADR.

## Documentation updates

- [x] §8 — adds 0179.
- [x] ADRs 0084 / 0174 / 0175 / 0176 / 0177 future-work — "Local source restriction" RESOLVED for the 4 covered positions.
- [x] Sweep retrospective — "Next moves" 0179 landed.

## Test count delta

```
Step 0: 1397 (after ADR 0178 refactor)
C1 (doc): 1397 → 1397
C2 (6 e2e Red Day 0): 1397 → 1397
C3 (HIR materialisation): 1397 → 1403
```

## Critical files

- `src/hir/mod.rs`:
  - Add `materialize_tagged_source_if_needed` helper.
  - Apply at IndexTagged construction site.
  - Apply at `lower_builtin_call` arg loop for rawget/rawset arg[1] and arg[2].
  - Drop the `Local-only` clauses from validation predicates.
- `src/codegen/emit.rs`:
  - Update `unreachable!()` / `UnsupportedExpr` messages at the 4 sites.
- `tests/phase2_6plus_non_local_tagged_source.rs` (NEW) — 6 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Materialisation fires on already-Local args (regression of IR shape) | Helper short-circuits when `expr.kind == Local(_)`; existing 1397 corpus catches deviation. |
| `pending_pre_stmts` ordering broken by mid-arg materialisation | `materialize_to_synth_local` precedent shows this pattern works for callee position; arg-position is identical. |
| `IndexTagged` widening collides with `widen_index_for_local_init` | Helper composes the same widener; no double-widening risk. |
| Synth local counter overflow | `usize`-typed (`callee_seq`); millions-of-synths would be required before overflow. |
| HIR validation no-longer-rejects something codegen still can't handle | The 4 sites all currently lower TaggedValue Local via existing dispatcher; the helper ensures the Local invariant. Codegen `unreachable!()` catches any contract violation in debug. |

## Future work

- IndexAssign value-side materialisation (the one remaining deferral row).
- Materialise non-TaggedValue source kinds where useful (e.g. Number key from a Call) — currently works but via a different mechanism.
- Synth local name collisions across nested non-trivial patterns — already unique by counter, but a debug-readable naming convention (e.g. `__tagged_src_at_line_NN`) could help log inspection.

## References

- [ADR 0084](0084-phase2-6plus-taggedvalue-key.md) — Local restriction precedent.
- [ADR 0091](0091-phase2-6plus-callee-norm.md) — `materialize_to_synth_local` original use.
- [ADR 0092](0092-phase2-6plus-methods.md) — helper rename / extension to MethodCall.
- [ADR 0139](0139-phase2-6plus-pairs-body-newindex.md) — slot-ptr-as-value convention.
- [ADR 0174](0174-rawget-tagged-key.md) / [0175](0175-rawset-tagged-key.md) / [0176](0176-rawset-tagged-key-tagged-value.md) / [0177](0177-index-tagged-key.md) — restriction-bearing ADRs.
- [ADR 0178](0178-tagged-key-runtime-dispatch-helper.md) — codegen dispatcher unification (prerequisite).
- [Sweep retrospective 0166-0177](../notes/sweep-0166-0177-retrospective.md) — anti-ad-hoc rationale.
