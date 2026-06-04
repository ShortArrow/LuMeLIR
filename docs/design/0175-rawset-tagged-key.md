# 0175. `rawset(t, k, v)` — Local(TaggedValue) Key, Non-TaggedValue Value

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-04
- **Deciders:** ShortArrow

## Context

[ADR 0174](0174-rawget-tagged-key.md) closed the read-side TaggedValue Local-key dispatch for `rawget`. This ADR mirrors the write side for `rawset`, with the same Local restriction. **TaggedValue VALUE is deferred** — the existing `emit_value_slot_store_dispatched` does not accept TaggedValue source, and routing that case correctly through both Number-key array path and hash-key path together is its own non-trivial scope.

## Scope (literal)

- ✅ `rawset(t, k, v)` where `k` is `HirExprKind::Local(LocalId)` with `kind == TaggedValue` AND `v` is non-TaggedValue (Number / String / Bool / Function / Table). Dispatches at runtime on the key slot's tag:
  - `TAG_NUMBER` → bitcast payload to f64, NaN trap, f2i, `key >= 1` trap, then `emit_array_index_assign_at` (ADR 0168 raw-write primitive — bypasses `__newindex` per `rawset` semantics).
  - `TAG_STRING / TAG_BOOL / TAG_FUNCTION / TAG_TABLE` → pass the source slot directly as `search_key_slot` to `emit_hash_indexassign_with_newindex` with `skip_metatable = true`.
  - `TAG_NIL` → trap (`s_table_index_nan` is wrong — use a fresh `s_table_index_nil_rawset` global, or reuse `s_table_oob`; this ADR reuses the existing NaN trap for symmetry with the static Number-key NaN guard).
- ✅ Returns `t` (Lua §6.1).
- ❌ TaggedValue VALUE — HIR rejects when key is Local(TaggedValue) AND value is TaggedValue. Sibling ADR scope.
- ❌ Non-Local TaggedValue source for key.
- ❌ Nil value — rejected as before (ADR 0136).

## Decision

### HIR validation (`src/hir/mod.rs`)

The `arg_idx == 1` raw-builtins check gains a `RawSet + Local(TaggedValue)` arm parallel to ADR 0174's. The value-side validation (`arg_idx == 2`) gains an additional rejection: when `builtin == RawSet` AND the FIRST other-arg (key) was Local(TaggedValue), AND the value is TaggedValue → reject with a message pointing to the future sibling ADR.

The simplest implementation reads back through `lowered_args` (already collected) to check whether arg[1] is Local(TaggedValue) at the time arg[2] is validated:

```rust
if matches!(builtin, Builtin::RawSet) && arg_idx == 2 {
    if matches!(k, ValueKind::TaggedValue) {
        let arg1_is_tagged_local = lowered_args.get(1).map(|a| {
            matches!(a.kind, HirExprKind::Local(_))
                && matches!(infer_kind(a, ...), ValueKind::TaggedValue)
        }).unwrap_or(false);
        if arg1_is_tagged_local {
            return Err(TypeMismatch { ... "non-tagged value (Number/String/..."} );
        }
    }
}
```

### Codegen (`src/codegen/emit.rs`)

`Callee::Builtin(Builtin::RawSet)` arm gains a TaggedValue-key sub-arm before the existing Number / hash dispatch. Mirror of ADR 0174:

```rust
if matches!(key_kind, ValueKind::TaggedValue) {
    let local_idx = match &args[1].kind { Local(LocalId(i)) => *i, _ => unreachable!() };
    let source_slot = slots[local_idx];
    let tag = load source_slot, i64;
    scf.if(tag == TAG_NUMBER) {
        let key_f64 = bitcast(load source_slot + 8);
        NaN trap; f2i; key >= 1 trap;
        emit_array_index_assign_at(t_ptr, key_i, value_v, value_kind);
    } else {
        emit_hash_indexassign_with_newindex(
            t_ptr, source_slot, /*key_kind=*/ Nil-sentinel,
            /*key_value=*/ unused, value_v, value_kind,
            METATABLE_INDEX_MAX_HOPS, /*skip_metatable=*/ true, ...);
    }
    return Ok(t_ptr);
}
// existing Number-key + hash-key arms unchanged
```

For the hash arm: the helper currently accepts a `key_kind` and `key_value` used to materialise the search slot. We pass the existing pre-built source slot via `search_key_slot` directly and pass `key_kind = TaggedValue` so the helper takes the "slot already prepared" path (matching ADR 0139's existing usage from IndexAssign).

## Alternatives considered

- **Bundle TaggedValue-value into this ADR**. Rejected — `emit_value_slot_store_dispatched` rejects TaggedValue source; supporting it correctly through the Number-key array path needs a tag-aware slot copy in `emit_array_index_assign_at`, which is sibling-ADR territory.
- **Reject only at HIR for the runtime-impossible combination**. Same as above; the rejection at HIR is the chosen approach.
- **Factor the Number-key sub-arm into a helper shared with ADR 0172/0173/0174**. Rejected — rule of three not yet paid; the fourth use will trigger extraction.

## Consequences

**Positive**
- pairs-body `for k, v in pairs(src) do rawset(dst, k, "literal") end` now compiles.
- Closes the rawset side of the "TaggedValue runtime-key dispatch" deferral row.

**Negative**
- pairs-body with TaggedValue value (`rawset(dst, k, v)`) still rejected. Documented.
- Three Number-key sub-arms now duplicate ~30 LOC each (ADRs 0173 / 0174 / 0175). Cleanup ADR queued.

**Locked in until superseded**
- Local restriction on key.
- Non-TaggedValue restriction on value.

## Documentation updates

- [x] §8 — adds 0175 (when SoT next refresh).
- [x] ADR 0174 future-work — sibling rawset bullet RESOLVED for non-TaggedValue value.

## Test count delta

```
Step 0: 1391 (after ADR 0174)
C2 (3 e2e Red Day 0): 1391 → 1391
C3 (impl): 1391 → 1394
```

## Critical files

- `src/hir/mod.rs`: extend `arg_idx == 1` AND `arg_idx == 2` checks.
- `src/codegen/emit.rs`: `Callee::Builtin(Builtin::RawSet)` arm gains a TaggedValue-key sub-arm.
- `tests/phase2_6plus_rawset_tagged_key.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Number-key + hash-key paths regress | TaggedValue sub-arm precedes existing static dispatch. |
| TaggedValue value silently accepted | HIR rejection at `arg_idx == 2` enforces the limit. Test 3 pins. |
| `__newindex` consulted | `skip_metatable = true` flag wired through; matches ADR 0136 contract. |

## Future work

- TaggedValue VALUE for tagged-key rawset (sibling ADR — needs `emit_array_index_assign_at` to accept TaggedValue source via raw 16-byte copy).
- Non-Local TaggedValue source for either rawset / rawget key.
- Same TaggedValue Local-key dispatch for `Index` / `IndexAssign` chokepoints.

## References

- [ADR 0084](0084-phase2-6plus-taggedvalue-key.md) — TaggedValue Local-key restriction precedent.
- [ADR 0136](0136-raw-set-get-builtins.md) — `rawset` / `rawget` builtins.
- [ADR 0168](0168-newindex-number-key-table-form.md) — `emit_array_index_assign_at` primitive.
- [ADR 0174](0174-rawget-tagged-key.md) — read-side sibling (algorithm mirrored).
- Lua 5.4 reference manual §6.1 — `rawset` spec.
