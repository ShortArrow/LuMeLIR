# 0171. Mid-Array `TAG_NIL` Slot Triggers Number-Key `__newindex`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-03
- **Deciders:** ShortArrow

## Context

ADRs 0168/0169/0170 closed Number-key `__newindex` for the `key > length` case (Table single-hop / Function / Table multi-hop). All three explicitly deferred the mid-array case: an in-range slot whose current tag is `TAG_NIL` should also fire `__newindex` per Lua 5.4 §2.4 (`__newindex` fires whenever `rawget` would return nil).

This ADR closes the last future-work bullet of the Number-key metatable matrix.

## Scope (literal)

- ✅ `t[i] = v` where `i in [1, length]` AND the current `array_buf[i]` slot tag is `TAG_NIL` → consult `mt.__newindex` (Table-form recurse + Function-form dispatch, both single + multi-hop via the existing helper).
- ✅ Both fresh-Nil slots (from gap-fill via ADR 0057) and explicitly-set-Nil slots (`t[3] = nil; t[3] = "x"`) qualify — they share the same `TAG_NIL` representation.
- ❌ Non-Number value Function form (inherited from ADR 0169).
- ❌ TaggedValue runtime-key dispatch.

## Decision

### Codegen

`emit_number_key_indexassign_routed` (introduced in ADR 0170) gets one new pre-check:

```
let probe_mid_nil: i1 = scf.if(key_high)
    .then(yield false)
    .else(
        let array_buf = emit_table_array_buf(target_ptr);
        let elem_ptr = emit_array_elem_ptr(array_buf, key_i);
        let tag = load(elem_ptr, i64);
        yield (tag == TAG_NIL)
    );
let trigger = key_high OR probe_mid_nil;
```

The subsequent metatable probe (`high_then` region) is guarded on `trigger` instead of `key_high`. Everything else (Table-form routing alloca, Function-form arm, depth recursion, final dispatch) is unchanged.

When `trigger == true` AND `mt.__newindex` is Table → route. When the inner write (recursive call) happens, the OUTER slot is **not** modified — same as ADR 0168's contract. The mid-array nil stays nil at the outer; the new value lands at the inner.

When `trigger == true` AND `mt.__newindex` is missing/non-Table/non-Function-candidate → fall through to `emit_array_index_assign_at` on outer. Identical to the no-trigger path (writes at elem_ptr; grow_if_needed is a no-op for in-range key).

### Why guard the tag load on `!key_high`

`emit_array_elem_ptr(array_buf, key_i)` with `key_i > length` accesses past the buffer (UB). The scf.if guard ensures the load only happens when `key_i ∈ [1, length]`.

## Alternatives considered

- **Always trigger on TAG_NIL regardless of length**. Rejected — out-of-range key already triggers via `key_high`; combining changes nothing for the OOB case, but the load would be unsafe.
- **Make `emit_array_index_assign_at` itself check**. Rejected — the helper is the raw-write primitive used at depth-0 fallback and at the final outer-write path; pushing the check into it would require it to know about `__newindex` state, breaking layering.
- **Only fire on slots that were never written (vs explicit set-to-nil)**. Rejected — Lua doesn't distinguish; both are `rawnil`.

## Consequences

**Positive**
- Lua-spec parity for Number-key `__newindex` rawnil semantics.
- Closes the last documented Number-key future-work bullet across ADRs 0168/0169/0170.

**Negative**
- Each Number-key write site loads one extra i64 (the slot tag) when in-range. Negligible.
- The pre-load of array_buf + elem_ptr inside the helper duplicates work that `emit_array_index_assign_at` will redo on the no-route path. Acceptable; the redundancy is hidden behind scf.if.

**Locked in until superseded**
- `TAG_NIL` is the only mid-array trigger. Other tag values do not consult `__newindex` on write (matches Lua spec — only rawnil triggers).

## Documentation updates

- [x] §8 — adds 0171 (when SoT next refresh).
- [x] ADR 0168/0169/0170 future-work — mid-array TAG_NIL trigger RESOLVED.

## Test count delta

```
Step 0: 1380 (after ADR 0170)
C2 (2 e2e Red Day 0): 1380 → 1380
C3 (impl): 1380 → 1382
```

## Critical files

- `src/codegen/emit.rs`:
  - `emit_number_key_indexassign_routed`: insert `probe_mid_nil` scf.if before the metatable probe; OR with `key_high` to form `trigger`.
- `tests/phase2_6plus_newindex_mid_array_nil.rs` (NEW) — 2 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| In-range existing-value writes regress (the common case) | `probe_mid_nil` returns false when slot tag is not TAG_NIL → `trigger == key_high == false` → fall through to existing outer write. Identical MLIR to before for non-nil overwrites. |
| Unsafe out-of-range load | Guarded by scf.if(key_high) — load only runs in the `key_in_range` branch. |
| Outer slot not nil-cleared after route | Lua spec: outer is untouched; the inner table holds the new value. Outer continues to read nil via rawget (slot tag stays TAG_NIL). Matches expectation. |

## Future work

- TaggedValue runtime-key trigger for `__newindex`.
- Mixed Table/Function multi-hop chains starting at a mid-array hit.
- `rawset(t, n, v)` Number-key path (ADR 0136 deferral).

## References

- [ADR 0168](0168-newindex-number-key-table-form.md) — Number-key Table form single-hop.
- [ADR 0169](0169-newindex-function-form-number-key.md) — Number-key Function form.
- [ADR 0170](0170-multi-hop-number-key-newindex.md) — multi-hop chain (helper this ADR extends).
- Lua 5.4 reference manual §2.4 — `__newindex` rawnil semantics.
