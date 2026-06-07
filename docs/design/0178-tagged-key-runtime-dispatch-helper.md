# 0178. Tagged-Key Runtime Dispatch Helper (Tidy First Refactor)

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-06-07
- **Deciders:** ShortArrow

## Context

Per the [sweep 0166-0177 retrospective](../notes/sweep-0166-0177-retrospective.md), the same `Local(TaggedValue)` runtime tag-dispatch scaffold is repeated inline in **3 sites** in `src/codegen/emit.rs`:

| Site | Line | Surfaced in |
|---|---|---|
| `emit_local_init_tagged` IndexTagged TaggedValue arm | ~5430 | ADR 0177 |
| `Callee::Builtin(Builtin::RawSet)` TaggedValue-key sub-arm | ~10963 | ADR 0175/0176 |
| `Callee::Builtin(Builtin::RawGet)` TaggedValue-key sub-arm | ~11191 | ADR 0174 |

Each site re-emits the same scf.if scaffold: load tag, compare against `TAG_NUMBER`, then-branch (bitcast i64→f64, NaN trap, f2i, site-specific Number arm), else-branch (site-specific hash arm). The scaffold body is ~50 LOC × 3 = ~150 LOC of inline duplication.

## Scope (literal)

- ✅ Extract a single helper `emit_tagged_key_runtime_dispatch` carrying the shared scaffold (tag load → cmpi → scf.if → bitcast/NaN-trap/f2i in the then-branch).
- ✅ Refactor 3 sites to call the helper with site-specific Number-arm and hash-arm closures.
- ✅ Preserve every observable behaviour — site-specific Nil-traps (IndexTagged only), too_small traps (rawset only), in-range scf.if (rawget only) stay at the call site outside the helper.
- ❌ No new behaviour, no new ADRs of consequence — pure Tidy First.
- ❌ No change to call sites that lack the runtime tag-dispatch shape (Number-key arms, hash-key arms, static-String arms).

## Decision

### New helper

```rust
fn emit_tagged_key_runtime_dispatch<'c, FN, FH>(
    context: &'c Context,
    block: &Block<'c>,
    source_slot: Value<'c, '_>,
    types: &Types<'c>,
    loc: Location<'c>,
    on_number_key: FN,
    on_hash_key: FH,
)
where
    FN: for<'a> FnOnce(&'a Block<'c>, Value<'c, 'a>),
    FH: for<'a> FnOnce(&'a Block<'c>),
```

Body (precisely the common scaffold):

```text
tag           = load source_slot, i64
tag_num_const = constant TAG_NUMBER : i64
is_num        = cmpi Eq, tag, tag_num_const
scf.if is_num {
  then:
    pay_ptr  = source_slot + ARRAY_ELEM_OFF_VALUE
    pay_i64  = load pay_ptr, i64
    key_f64  = bitcast i64 → f64
    is_nan   = cmpf Une, key_f64, key_f64
    trap_if(is_nan, "s_table_index_nan")
    key_i    = f2i key_f64
    on_number_key(then_blk, key_i)
    yield
  else:
    on_hash_key(else_blk)
    yield
}
```

### Per-site fences kept outside the helper

| Site | Stays at call site |
|---|---|
| IndexTagged | Nil-trap (`s_table_index_nil`) on `tag == TAG_NIL` BEFORE the dispatch |
| rawset | `too_small` trap (`s_table_oob`) inside the Number-arm closure |
| rawget | length-load + in-range scf.if inside the Number-arm closure |

These are kept outside because they are not shared. The helper only owns the shape that **is** shared.

### Why HRTB on the closures

The inner `then_blk` / `else_blk` constructed by the helper has a lifetime tied to the helper's stack, not the caller's. Higher-rank trait bounds (`for<'a> FnOnce(&'a Block<'c>, Value<'c, 'a>)`) let the caller pass a closure whose inner-block lifetime is universally quantified — the standard Rust pattern for "callback receives a borrow whose lifetime I create."

### Tests

No test delta. Existing 1399 tests pin every dispatcher site (ADRs 0174, 0175, 0176, 0177 e2e plus the cross-cutting suites).

## Alternatives considered

- **Enum-driven helper** (`enum TaggedKeyOp { RawGet { out_slot, t_ptr }, RawSet { value_v, value_kind, t_ptr }, IndexTagged { dst_slot, target_ptr, functions } }`). Rejected — pushes dispatch INTO the helper, expands its surface every time a new consumer arrives. Closures keep the helper tight and additive.
- **No extraction; document only**. Rejected — retrospective already documented; the next deferral row (Non-Local TaggedValue source) explicitly needs a single insertion point per ADR 0178's roadmap.
- **Per-side helpers** (`emit_read_tagged_dispatch` + `emit_write_tagged_dispatch`). Rejected — the shared scaffold is identical across read and write; the divergence is in the arm bodies, exactly what closures abstract.

## Consequences

**Positive**
- ~120 LOC removed from inline duplication.
- Next ADR (0179 = Non-Local TaggedValue source) needs to touch one call site (the helper's entry point) instead of three.
- Future "Function-form metamethod on tagged key" / "TaggedValue NaN trap policy refinement" land in one place.

**Negative**
- Adds one HRTB-quantified function — marginally harder to read for engineers unused to `for<'a>` on closure trait bounds.
- Closure capture rules require `Value` etc. to be `Copy` (they already are in melior).

**Locked in until superseded**
- The scaffold's exact shape (Nil-trap stays outside, NaN-trap stays inside the Number arm) — if a future ADR needs to move the NaN trap outside, that's a deliberate change.

## Documentation updates

- [x] [sweep retrospective](../notes/sweep-0166-0177-retrospective.md) — "Next moves" → 0178 = this ADR (lands the helper).
- [x] §8 — adds 0178.

## Test count delta

```
Step 0: 1399 (after ADR 0177)
C1 (doc): 1399 → 1399
C2 (refactor): 1399 → 1399 (behaviour-preserving)
```

## Critical files

- `src/codegen/emit.rs`:
  - NEW `emit_tagged_key_runtime_dispatch` (~70 LOC).
  - 3 call sites collapse from ~50 LOC scaffold each to ~10 LOC closure-pair invocation.
- `docs/notes/sweep-0166-0177-retrospective.md` — annotate 0178 landing.

## Risks

| Risk | Mitigation |
|---|---|
| Lifetime annotations on HRTB closures don't compile | Fallback: split into pure function emitting raw scaffold + per-site post-scf.if mutation (drops closures, requires duplicating yield boilerplate; ~30 LOC saving instead of ~120). |
| Closure capture of `Value` accidentally moves out of caller scope | `Value<'c, '_>` is `Copy` — captured by value, no move-out. |
| Refactor silently changes IR | The 3 e2e suites pin every observable behaviour; CI catches divergence. |
| Future ADR forgets the helper exists and re-inlines | Cross-reference in this ADR + retrospective doc; ADR 0179 explicitly plumbs through the helper. |

## Future work

- **ADR 0179** — Non-Local TaggedValue source (`t[f()]`, `rawget(t, f())`, ...): tmp-slot materialisation before the helper's entry, no per-site changes downstream.
- **Flat-f64 Number-only `Index` consumer widening** — orthogonal; may eventually flow through the same helper if widened to TaggedValue.
- **`__index` Function form on TaggedValue key** — extends Number arm and hash arm at the call site, not the helper.

## References

- [Sweep retrospective 0166-0177](../notes/sweep-0166-0177-retrospective.md) — duplication identification.
- [ADR 0084](0084-phase2-6plus-taggedvalue-key.md) — Local restriction precedent.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot`.
- [ADR 0174](0174-rawget-tagged-key.md) / [0175](0175-rawset-tagged-key.md) / [0176](0176-rawset-tagged-key-tagged-value.md) / [0177](0177-index-tagged-key.md) — 3 sites being unified.
- [ADR 0139](0139-phase2-6plus-pairs-body-newindex.md) — slot-ptr-as-value convention preserved.
