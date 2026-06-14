# 0187. IndexAssign Value-Side TaggedValue Source — HIR Materialisation + Codegen Arm

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-14
- **Deciders:** ShortArrow

## Context

[ADR 0179](0179-non-local-tagged-source-materialisation.md) closed the non-Local TaggedValue source gap at three RawGet / RawSet builtin-arg positions by adding a HIR materialisation pass (`materialize_tagged_source_if_needed`) that pre-binds non-Local TaggedValue producers (Call returns) into synth locals before lowering reaches the dispatcher sites. The symmetric write-path counterpart — `t[k] = <TaggedValue expr>` (`HirStmtKind::IndexAssign` with TaggedValue value) — was the original Day-0 candidate for ADR 0181 but pivoted to parameter String-context inference, with the IndexAssign work deferred.

Probe (commit-less, removed after) confirms the actual current state is broader than "non-Local source only":

```lua
local function pick(b) if b then return 1 end; return nil end
local t = {}
t.x = pick(true)   -- non-Local TaggedValue value
print(t.x)         -- → panics in codegen at emit.rs:3980 `unreachable!`

local v = pick(true)
t.x = v            -- Local TaggedValue value
print(t.x)         -- → also panics in codegen at emit.rs:3980 `unreachable!`
```

The HIR accepts a TaggedValue value when the key is non-Number (`src/hir/mod.rs:3450-3451`) but the codegen static-key arm (String / Bool / Function / Table key, lines 3771-3984) has no `ValueKind::TaggedValue` value branch — the wildcard `_ => unreachable!()` at line 3980 fires for both Local and non-Local TaggedValue values. The TaggedValue-key arm (lines 3993-4148, ADR 0138-M) already handles TaggedValue values for its case via slot-ptr substitution; this ADR mirrors that to the static-key arm and closes the non-Local source gap on the IndexAssign side via the existing `materialize_tagged_source_if_needed` helper.

## Scope (literal)

- ✅ HIR: in `LowerCtx::lower_stmt`'s `IndexAssign` arm (`src/hir/mod.rs:3389`), after `value_hir` is computed, route it through `materialize_tagged_source_if_needed(value_hir, value.span)?` so non-Local TaggedValue values pre-bind into a synth Local. Idempotent on shapes that do not need materialisation.
- ✅ Codegen: in the static-key (String / Bool / Function / Table) match at `src/codegen/emit.rs:3786-3984`, add a `ValueKind::TaggedValue` arm next to the existing concrete-kind arm. Body extracts the Local's slot ptr (mirror of lines 4022-4036), then routes through the shared `emit_hash_indexassign_with_newindex` with `value_kind = TaggedValue` and `value_v = slots[idx]`.
- ✅ 3 e2e (genuine Red Day 0, both Local and non-Local source fail today):
  1. Local TaggedValue value source on String key.
  2. Non-Local (Call-return) TaggedValue value source on String key.
  3. Boundary: assigning `nil` via a TaggedValue source whose runtime tag happens to be Nil (still hash-soft-delete semantics — value path with concrete kind unchanged).
- ❌ Number-key (array) + TaggedValue value. Rejected at HIR (`src/hir/mod.rs:3450-3451` requires `key_kind != Number` for TaggedValue value). Out of scope; staying consistent with ADR 0135's Number-key array path policy.
- ✅ `IndexTagged` target-side TaggedValue value (the deeper nested write `a.b.c = pick(...)`). **Post-impl audit (2026-06-14)** confirmed the IndexAssign codegen normalises both `Index` and `IndexTagged` targets through `emit_resolve_table_target_ptr` (`src/codegen/emit.rs:3629`) before reaching the static-key value-kind match — so this ADR's `ValueKind::TaggedValue` arm covers nested targets at any depth. Tests 4 and 5 in the e2e file pin single- and double-level nesting.
- ❌ Number-key TaggedValue-value codegen reachability check at `emit.rs:4286`. That `unreachable!` is downstream of the TaggedValue-key arm's `if !matches!(value_kind_early, Nil) { ...; return Ok(()); }` early-return; non-Nil values never reach the match. No change needed; documented here for traceability.

## Decision

### `src/hir/mod.rs`

In the `IndexAssign` arm of `lower_stmt`, between the existing `let value_hir = self.lower_expr(value)?;` and the kind / type checks, route through the materialisation helper:

```rust
let value_hir = self.lower_expr(value)?;
// ADR 0187 — pre-materialise non-Local TaggedValue values into
// a synth local so codegen's static-key TaggedValue-value arm
// (which expects a Local source for the 16-byte slot copy) sees
// a uniform shape. Idempotent for already-Local and non-Tagged
// sources.
let value_hir = self.materialize_tagged_source_if_needed(value_hir, value.span)?;
```

No other HIR change. The kind / type / closure-escape checks downstream see the post-materialised expression; their decisions stay correct because `materialize_tagged_source_if_needed` preserves `ValueKind::TaggedValue` (the synth local is declared TaggedValue).

### `src/codegen/emit.rs`

In the static-key match (`src/codegen/emit.rs:3786-3984`), insert a `ValueKind::TaggedValue` arm directly after the existing concrete-kind arm and before `ValueKind::Nil`. Mirror of the TaggedValue-key arm's TaggedValue-value handling (lines 4022-4051):

```rust
ValueKind::TaggedValue => {
    // ADR 0187 — TaggedValue value on a String / Bool / Function /
    // Table key. HIR has already materialised non-Local sources
    // into a synth local (`materialize_tagged_source_if_needed`),
    // so `value.kind` is `Local(LocalId)` here. Hand the local's
    // slot ptr to the shared helper, which performs the raw
    // 16-byte tagged-slot copy at commit time.
    let value_v_for_helper = match &value.kind {
        HirExprKind::Local(LocalId(idx)) => slots[*idx],
        _ => unreachable!(
            "ADR 0187 — HIR materialises non-Local TaggedValue values into a synth local; \
             non-Local source must not reach codegen"
        ),
    };
    emit_hash_indexassign_with_newindex(
        context,
        block,
        target_ptr,
        key_slot,
        key_kind,
        key_value,
        value_v_for_helper,
        ValueKind::TaggedValue,
        METATABLE_INDEX_MAX_HOPS,
        false,
        functions,
        types,
        loc,
    );
}
```

The wildcard `_ => unreachable!` at line 3980 stays — it now only catches `Number` value with non-Number key (HIR-impossible) and is the true last-resort guard.

### Tests

`tests/phase2_6plus_indexassign_value_tagged.rs` (NEW, 3 e2e):

1. **Local TaggedValue value source, String key**: `local v = pick(true); t.x = v; print(t.x)` → `1`.
2. **Non-Local TaggedValue value source, String key**: `t.x = pick(true); print(t.x)` → `1`.
3. **Local TaggedValue source whose runtime tag is Nil (hash-delete via TaggedValue)**: `local v = pick(false); t.x = v; print(t.x == nil)` → `true`. Verifies the slot-copy path correctly propagates the Nil tag (no special-case needed because the runtime tag dispatches inside `emit_hash_indexassign_with_newindex`).

All three are Red Day 0 (probe confirmed both Local and non-Local sources currently panic at `unreachable!`).

## Alternatives considered

- **Codegen-only fix (no HIR materialisation).** Rejected — would leave non-Local sources rejected via `CodegenError::UnsupportedExpr` (the precedent at the TaggedValue-key arm lines 4026-4033). The HIR materialisation pre-binds uniformly per the ADR 0179 retrospective insight (sweep 0166-0177 memo §Horizontal duplication): opening codegen one site at a time re-introduces the ad-hoc shape the materialisation chokepoint just collapsed.
- **HIR rejection of TaggedValue value.** Rejected — HIR currently accepts it (line 3450-3451). Removing the acceptance would close idiomatic Lua patterns that already work in other dispatchers (pairs loop body's `t[k] = v` per ADR 0138-M).
- **New helper dedicated to IndexAssign value materialisation.** Rejected — `materialize_tagged_source_if_needed` from ADR 0179 is shape-correct already (kind check + Local check + synth-local synth). Reuse, not parallel helpers.
- **Bundle the `IndexTagged` target-side gap into this ADR.** Rejected — needs codegen audit (the `IndexTagged` write path may dispatch through a different match); separate ADR keeps the surface reviewable.

## Consequences

**Positive**
- `t[k] = <Call returning TaggedValue>` and `t[k] = <Local TaggedValue>` both compile and run on String / Bool / Function / Table keys.
- Codegen's TaggedValue-value handling is symmetric between the TaggedValue-key arm (ADR 0138-M) and the static-key arm (this ADR).
- One more "Local-only" scope clause from ADRs 0084 / 0174 / 0175 / 0176 / 0177 retires for the IndexAssign value position.

**Negative**
- Codegen adds one `match` arm + 18 LOC. Surface increase minimal.
- HIR's IndexAssign arm gains a single line — same cost the RawGet / RawSet arms paid in ADR 0179.

**Locked in until superseded**
- `materialize_tagged_source_if_needed` remains the SoT for non-Local TaggedValue → synth-Local conversion. Any new dispatcher with the same pattern (e.g. a future `IndexTagged` target write site) routes through it.

## Documentation updates

- [x] §8 — adds 0187.
- [x] ADR 0179 future-work — IndexAssign value-side covered here; remaining non-Local TaggedValue source gaps now live with ADR 0188+ candidates (if discovered).

## Test count delta

```
Step 0: 1418 (after 485d4f9)
C1 (doc): 1418 → 1418
C2 (3 e2e Red Day 0): 1418 → 1418 (Red)
C3 (HIR + codegen impl): 1418 → 1421 (Green)
```

## Critical files

- `src/hir/mod.rs`:
  - Add `materialize_tagged_source_if_needed` call in IndexAssign lower_stmt arm (line ~3398).
- `src/codegen/emit.rs`:
  - Add `ValueKind::TaggedValue` arm in the static-key value-kind match at lines 3786-3984.
- `tests/phase2_6plus_indexassign_value_tagged.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| HIR materialisation fires when value is Local TaggedValue, growing the IR unnecessarily | `materialize_tagged_source_if_needed` is idempotent on `HirExprKind::Local(_)` — Local sources pass through unchanged. |
| `value_v` was previously computed for non-Tagged values; replacing it at value-kind=TaggedValue could miss a side-effect | Probed both shapes (Local + Call). The current `emit_expr` on a Call returning TaggedValue traps at `unreachable!` so no side-effect is in play; for Local TaggedValue the existing `value_v` was equivalently never reachable (also traps). Both are pure introductions. |
| `IndexTagged` target-side same gap remains uncovered | Documented as scope ❌ + future-work; verifiable via the same probe technique when raised. |
| `emit_hash_indexassign_with_newindex` does not support TaggedValue value in the static-key path | Existing TaggedValue-key arm already calls it with `value_kind_early = ValueKind::TaggedValue` (line 4045) — the helper is value-kind-agnostic. |
| The renamed synth local name `__tagged_src_N` collides with another sequence | `callee_seq` is module-monotonic per ADR 0179; no collision. |

## Future work

- ~~`IndexTagged` (nested write) target-side TaggedValue value~~ — RESOLVED by post-impl audit (see Scope row above); shared codegen path covers all nesting depths.
- Source-position TaggedValue normalisation across any remaining dispatcher with a `Local`-only scope-ceiling clause (the ADR 0179 retrospective predicts more lurking).
- Cross-procedure TaggedValue kind inference (caller refines callee body) — orthogonal but related.

## References

- [ADR 0084](0084-phase2-8e-iter-tk.md) — TaggedValue-key dispatcher introduction (first Local-only scope ceiling).
- [ADR 0095](0095-nested-index-assign-widen.md) — nested IndexAssign Index→IndexTagged widen (target side).
- [ADR 0138](0138-phase2-7y-tier1-metatables-newindex.md) — TaggedValue value on TaggedValue key (precedent for the slot-ptr substitution this ADR mirrors).
- [ADR 0179](0179-non-local-tagged-source-materialisation.md) — `materialize_tagged_source_if_needed` chokepoint reused here.
- [`docs/notes/sweep-0166-0177-retrospective.md`](../notes/sweep-0166-0177-retrospective.md) — Horizontal-duplication insight motivating the choke-point reuse.
