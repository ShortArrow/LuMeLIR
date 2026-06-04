# 0177. `t[k]` — Local(TaggedValue) Key (Tagged-Consumer Path)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-05
- **Deciders:** ShortArrow

## Context

`local x = t[k]` where `k` is a TaggedValue Local (typical pairs-body `for k, v in pairs(t1) do x = t2[k] end`) currently trips an `unreachable!("IndexTagged key must be Number or String")` inside `emit_local_init_tagged`'s IndexTagged arm (`src/codegen/emit.rs:5488`). [ADR 0174](0174-rawget-tagged-key.md) wired this dispatch for rawget; this ADR brings the same algorithm to the IndexTagged chokepoint used by the language-level `t[k]` syntax flowing into a tagged consumer.

This ADR's tagged-consumer scope mirrors what [ADR 0165](0165-number-key-index-array-oob-fallback.md) noted for the Number-key reader: only the TaggedValue-widening consumers (`local x = t[k]`, `print(t[k])` post lowering, etc.) get the new behaviour. The flat-f64 `Index` path at `emit_expr` is untouched.

## Scope (literal)

- ✅ `t[k]` where `t` is a Table-typed Local (or IndexTagged-narrowed target) AND `k` is `Local(TaggedValue)` AND the consumer demands TaggedValue (LocalInit/Assign of TaggedValue local). Runtime tag dispatch:
  - `TAG_NUMBER` → bitcast payload to f64, run the existing Number-key array sub-arm including ADR 0165/0167 `__index` fallback.
  - `TAG_STRING / TAG_BOOL / TAG_FUNCTION / TAG_TABLE` → pass source slot as `search_key_slot`, run the existing hash sub-arm including ADR 0134/0150 `__index` chain.
  - `TAG_NIL` → trap (`s_table_index_nil`) — Lua spec §3.4.10.
- ❌ Non-Local TaggedValue source (e.g. `t[f()]` where `f` returns TaggedValue) — same restriction as ADR 0084 / 0174 / 0175.
- ❌ Flat-f64 Number-only `Index` consumer at `emit_expr` (still rejects TaggedValue key).
- ❌ IndexAssign side (`t[k] = v`) — already covered by ADR 0139 / 0084.

## Decision

### Codegen (`src/codegen/emit.rs`)

`emit_local_init_tagged`'s IndexTagged `match key_kind` (line ~5300+) gains a TaggedValue arm before the `_ => unreachable!` floor:

```rust
ValueKind::TaggedValue => {
    let local_idx = match &key.kind {
        HirExprKind::Local(LocalId(i)) => *i,
        _ => return Err(CodegenError::UnsupportedExpr(
            "IndexTagged TaggedValue key requires Local source (ADR 0177 scope)"
        )),
    };
    let source_slot = slots[local_idx];
    let tag = load source_slot, i64;
    // Nil-trap: t[nil] is a Lua error.
    let is_nil = (tag == TAG_NIL);
    emit_trap_if(block, is_nil, "s_table_index_nil");
    // Dispatch.
    let is_num = (tag == TAG_NUMBER);
    scf.if(is_num) {
        let key_f64 = bitcast(load source_slot + 8);
        // Run the existing Number-key array sub-arm (ADR 0165/0167):
        //   NaN trap, f2i, in-range copy, OOB → emit_number_key_metatable_index_fallback.
    } else {
        // Hash sub-arm (ADR 0088/0134/0150):
        //   emit_hash_lookup_into_tagged_slot + emit_metatable_index_fallback_if_nil.
        emit_hash_lookup_into_tagged_slot(target_ptr, source_slot, dst_slot, NilOnMissing);
        emit_metatable_index_fallback_if_nil(target_ptr, source_slot, dst_slot, ...);
    };
}
```

### New global

A new `s_table_index_nil` string global is reused if one already exists (per ADR 0086 NaN trap precedent); otherwise added once. The diagnostic text mirrors Lua's "table index is nil".

### Why duplicate the Number-key OOB fallback block

The existing Number-key arm (line ~5300-5446) is inline. ADR 0177's Number-tag branch needs to run that same code with a different f64 source (bitcast i64 vs. emit_expr f64). Two options:
1. Copy/paste the block (~140 LOC).
2. Extract a helper.

Per Tidy First and the third-use rule (ADRs 0173/0174/0177 all need a "Number-key in-range copy then OOB __index fallback" block), this ADR extracts a small helper `emit_number_key_indextagged_lookup(target_ptr, key_i, dst_slot, ...)` that does:
- in-range scf.if + raw 16-byte copy
- OOB → emit_value_slot_store_nil + emit_number_key_metatable_index_fallback

The existing inline Number-key arm refactors to call this helper. Both arms call the same code; future ADRs touching the read fallback need only modify the helper.

## Alternatives considered

- **Non-Local TaggedValue via tmp materialisation**. Rejected — out of scope per ADR-per-decision.
- **No helper extraction**. Rejected — third use justifies the helper (rule of three).
- **Forward-thread the IndexAssign-side restriction**. Rejected — IndexAssign already works via ADR 0139.

## Consequences

**Positive**
- `local x = t[k]` works for `Local(TaggedValue)` key.
- Closes the IndexTagged-side gap that's been a "TaggedValue runtime-key dispatch" deferral row across ADRs 0142/0144/0146/0147/0150/0166/0171/0173.
- Extracted helper centralises the Number-key in-range copy + `__index` fallback used by the Number-key reader and the TaggedValue reader.

**Negative**
- Helper extraction touches the existing inline arm. Risk mitigated by the existing Number-key OOB regression tests.
- Non-Local TaggedValue source still rejects with `UnsupportedExpr`. Documented.

**Locked in until superseded**
- Local restriction on source.

## Documentation updates

- [x] §8 — adds 0177 (when SoT next refresh).
- [x] ADR 0150 / 0165 / 0166 / 0171 / 0173 future-work — "TaggedValue runtime-key dispatch" RESOLVED for Local source (read-side; write side already covered by ADR 0139).

## Test count delta

```
Step 0: 1396 (after ADR 0176)
C2 (3 e2e Red Day 0): 1396 → 1396
C3 (impl): 1396 → 1399
```

## Critical files

- `src/codegen/emit.rs`:
  - Extract helper `emit_number_key_indextagged_lookup` (~80 LOC; move of the existing inline body of the IndexTagged Number-key arm).
  - Add TaggedValue arm dispatching to that helper (Number tag) or `emit_hash_lookup_into_tagged_slot` (other tags).
  - Add `s_table_index_nil` global (one definition).
- `tests/phase2_6plus_index_tagged_key.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Number-key reader regresses | Helper extraction is verbatim move; existing tests pin. |
| Hash-key reader regresses | TaggedValue arm precedes static dispatch; static paths untouched. |
| Nil-key trap message wording | Use `s_table_index_nil` (new); if a global with similar semantics already exists, reuse. |
| __index chain entered with wrong key kind | Both arms pass the same kind-shaped slot the existing fallbacks expect. |

## Future work

- Non-Local TaggedValue source.
- Flat-f64 Number-only `Index` consumer widening to TaggedValue.
- Same dispatch for `IsNil(t[k])` and `tostring(t[k])` if not already lowering through IndexTagged.

## References

- [ADR 0084](0084-phase2-6plus-taggedvalue-key.md) — TaggedValue Local restriction precedent.
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot`.
- [ADR 0134](0134-metatables-index-read.md) — `__index` hash-key fallback (preserved at each hop).
- [ADR 0165](0165-number-key-index-array-oob-fallback.md) — Number-key `__index` fallback (helper centralises this).
- [ADR 0174](0174-rawget-tagged-key.md) — rawget mirror.
- Lua 5.4 reference manual §3.4.10 — table indexing.
