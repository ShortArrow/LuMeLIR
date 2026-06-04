# 0176. `rawset(t, k, v)` — Tagged Key + Tagged Value (Full Pairs-Body)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-05
- **Deciders:** ShortArrow

## Context

[ADR 0175](0175-rawset-tagged-key.md) wired tagged-key rawset for non-TaggedValue values, leaving the canonical pairs-body shape `for k, v in pairs(src) do rawset(dst, k, v) end` (both `k` and `v` are TaggedValue Locals) rejected. This ADR closes that gap.

The blocker was `emit_value_slot_store_dispatched` (the array-write tail of `emit_array_index_assign_at`) rejecting TaggedValue source. ADR 0139 already established the convention for the hash path: when `value_kind == TaggedValue`, treat `value_v` as a slot ptr and do raw 16-byte copy. This ADR extends the same convention to the Number-key array path.

## Scope (literal)

- ✅ `rawset(t, k, v)` where `k` is `Local(TaggedValue)` AND `v` is `Local(TaggedValue)`. Both Number-tag and hash-tag dispatch paths preserved.
- ✅ Reuses ADR 0139 convention (TaggedValue value_v = slot ptr) for both arms.
- ❌ Non-Local TaggedValue value (e.g. `rawset(t, k, f())` where `f` returns TaggedValue) — needs tmp materialisation, separate ADR.
- ❌ Index / IndexAssign chokepoints — already covered by ADR 0139; this ADR is rawset-specific.

## Decision

### `emit_array_index_assign_at` (`src/codegen/emit.rs`)

The store tail dispatches on `value_kind`:

```rust
let elem_ptr = emit_array_elem_ptr(...);
match value_kind {
    ValueKind::TaggedValue => {
        // ADR 0176 — value_v is a slot ptr (Local(TaggedValue)
        // source). Raw 16-byte copy preserves the tag.
        emit_copy_tagged_slot_16b(context, block, value_v, elem_ptr, types, loc);
    }
    _ => {
        emit_value_slot_store_dispatched(context, block, elem_ptr, value_v, value_kind, types, loc);
    }
}
```

This composes cleanly with ADRs 0168/0170/0171 (which all call through `emit_array_index_assign_at`); IndexAssign with TaggedValue value now also works through the array path when route conditions don't fire (rare in practice — `t[i] = v` with i: Number already uses the Number-key arm, which calls into the routed helper, but value_v was previously emit_expr'd; for `Local(TaggedValue)` value, we need the same slot-substitution treatment).

### HIR validation (`src/hir/mod.rs`)

The ADR 0175 rejection of TaggedValue value when key is `Local(TaggedValue)` is **narrowed**: rejection now requires that the value also NOT be `Local(TaggedValue)`. Local-TaggedValue value joins the allowed set.

```rust
if matches!(builtin, Builtin::RawSet) && arg_idx == 2 && matches!(k, ValueKind::TaggedValue) {
    let value_is_tagged_local = matches!(arg.kind, HirExprKind::Local(_));
    if !value_is_tagged_local {
        // existing TaggedValue rejection (non-Local source)
        return Err(...);
    }
}
```

### Codegen rawset TaggedValue-key sub-arm (`src/codegen/emit.rs`)

The ADR 0175 sub-arm reads `value_v` from `emit_expr`. For `Local(TaggedValue)` value source, substitute `slots[value_idx]` before calling either arm — same pattern as ADR 0139 line 3997-4011.

The Number-tag branch passes the substituted `value_v` (slot ptr) and `value_kind == TaggedValue` to `emit_array_index_assign_at`; the helper's new TaggedValue arm does the slot copy at the array element.

The hash-tag branch already handles `value_kind == TaggedValue` via `emit_hash_indexassign_with_newindex` (per ADR 0139).

### IndexAssign Number-key arm — also benefits

The Number-key `IndexAssign` arm computes `value_v` via emit_expr; for `Local(TaggedValue)` value it was already substituted at line 3997-4011 (TaggedValue-key path only). This ADR's `emit_array_index_assign_at` extension means: any future Number-key IndexAssign that emerges with `value_kind == TaggedValue` (e.g. through the routing helper at depth N) writes correctly. No fragile assumption that "TaggedValue value only reaches the hash path".

## Alternatives considered

- **Separate helper `emit_array_index_assign_at_tagged_source`**. Rejected — single `match value_kind` inside the existing helper is two lines and avoids parallel-helper drift.
- **Materialise TaggedValue value into a tmp Number-kind slot**. Rejected — would lose the tag, breaking Lua semantics.
- **Lift the non-Local restriction here too**. Rejected per ADR-per-decision; non-Local TaggedValue source is its own scope.

## Consequences

**Positive**
- Canonical pairs-body `for k, v in pairs(src) do rawset(dst, k, v) end` now compiles end-to-end.
- `emit_array_index_assign_at` becomes value-kind-complete (every tagged kind, including TaggedValue source, handled).
- The IndexAssign Number-key path through `emit_number_key_indexassign_routed` is also future-proofed.

**Negative**
- `emit_array_index_assign_at` grows a small dispatch branch (2 LOC). Trivial.
- HIR validation reads back two args (key + value) to make the joint decision. Still O(1).

**Locked in until superseded**
- Local restriction on value source.

## Documentation updates

- [x] §8 — adds 0176 (when SoT next refresh).
- [x] ADR 0175 future-work — TaggedValue VALUE bullet RESOLVED for Local source.

## Test count delta

```
Step 0: 1394 (after ADR 0175)
C2 (2 e2e Red Day 0): 1394 → 1394
C3 (impl): 1394 → 1396
```

## Critical files

- `src/codegen/emit.rs`: `emit_array_index_assign_at` store tail dispatches on `value_kind`; rawset TaggedValue-key sub-arm substitutes `slots[idx]` for Local-TaggedValue value.
- `src/hir/mod.rs`: narrow the ADR 0175 TaggedValue-value rejection to non-Local source.
- `tests/phase2_6plus_rawset_tagged_key_tagged_value.rs` (NEW) — 2 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| ADR 0168/0170/0171 routed Number-key paths break | Helper change is additive; existing callers always pass non-TaggedValue value_kind (HIR rejects TaggedValue value at IndexAssign in scopes covered by those ADRs). The new arm only activates when value_kind == TaggedValue, which currently only fires through the new rawset sub-arm. |
| TaggedValue value with wrong slot convention | The slot-ptr substitution mirrors ADR 0139 exactly; same convention. |
| Non-Local TaggedValue value silently flows through | HIR rejection narrowed but still in place for non-Local case. |

## Future work

- Non-Local TaggedValue source for either rawset key or value.
- Same Local(TaggedValue) key dispatch for `Index` chokepoint (rawget already has it via ADR 0174; the static `t[k]` Index expr is its own scope).
- Mark ADR 0139 line 3997-4011 substitution dead-code candidate if the helper-side handling subsumes it.

## References

- [ADR 0139](0139-taggedvalue-key-newindex-wiring.md) — TaggedValue key + value convention for the hash path.
- [ADR 0168](0168-newindex-number-key-table-form.md) — `emit_array_index_assign_at` extraction.
- [ADR 0175](0175-rawset-tagged-key.md) — non-TaggedValue value sibling.
- Lua 5.4 reference manual §6.1 — `rawset` spec.
