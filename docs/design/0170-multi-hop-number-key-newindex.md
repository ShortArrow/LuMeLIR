# 0170. Multi-Hop Number-Key `__newindex` (Table-Form Chain)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-02
- **Deciders:** ShortArrow

## Context

[ADR 0167](0167-multi-hop-number-key-index.md) closed the read-side multi-hop. [ADR 0168](0168-newindex-number-key-table-form.md) introduced single-hop Table-form Number-key `__newindex`, but the inner write was explicitly "raw" — it did not consult the inner table's own `mt.__newindex`. This ADR closes that gap: a chained `__newindex = Table` walk that mirrors ADR 0167's depth-recursion model.

## Scope (literal)

- ✅ Multi-hop Number-key `__newindex` with Table-form chain links. Each hop's `key > current length` triggers re-check of that table's metatable; if Table, recurse; if Function, dispatch and stop.
- ✅ Reuses `METATABLE_INDEX_MAX_HOPS` (shared with ADR 0167).
- ✅ Function-form arm (ADR 0169) fires at any hop and terminates.
- ✅ Budget-exhaustion path: at depth 0 the recursion falls through to a raw `emit_array_index_assign_at` on the current target (the inner-most table reached). Reasonable Lua-spec approximation; matches the "best-effort cycle guard" stance of ADR 0167.
- ❌ Mid-array `TAG_NIL` slot trigger (still raw-writes).
- ❌ Non-Number value Function form.
- ❌ TaggedValue runtime-key dispatch.

## Decision

### Codegen

The Number-key `IndexAssign` arm's routing logic (currently inline, ~250 LOC across ADRs 0168/0169) is **Tidy First refactored** into a new helper:

```rust
fn emit_number_key_indexassign_routed(
    context, block,
    target_ptr,           // table descriptor ptr to write into
    key_i, value_v,
    value_kind,
    remaining_hops: u32,
    functions, types, loc,
)
```

Body = the existing route_iv + handled_by_fn alloca dance + scf.if dispatch. **In the route_then branch (Table form), instead of calling `emit_array_index_assign_at(inner_target, ...)` directly, recurse: `emit_number_key_indexassign_routed(inner_target, key_i, value_v, value_kind, remaining_hops - 1, ...)`**.

Depth-0 early-return: at the very top of the helper, if `remaining_hops == 0`, call `emit_array_index_assign_at(target_ptr, ...)` immediately and return (raw write, breaks the static unroll).

The `IndexAssign` Number-key arm calls the helper with `remaining_hops = METATABLE_INDEX_MAX_HOPS`.

## Alternatives considered

- **Trap on budget exhaustion**. Rejected — write side shouldn't crash on chains within static budget; matches ADR 0167's "Number-key OOB → silent fallback" stance.
- **Noop on budget exhaustion**. Rejected — the user requested a write; falling through to raw write at the inner-most reached table is the least surprising option.
- **Combine with mid-array TAG_NIL detection**. Rejected per ADR-per-decision.

## Consequences

**Positive**
- Chained Table-form Number-key `__newindex` resolves (up to `METATABLE_INDEX_MAX_HOPS`).
- Closes ADR 0168 multi-hop future-work bullet.
- Pattern symmetry with ADR 0167 (read-side chain).
- Number-key metatable matrix now complete for Table + Function single/multi-hop, write/read sides (modulo mid-array Nil case).

**Negative**
- Each `IndexAssign` site now statically unrolls n levels of routing. At depth 8, code size grows; single call site keeps the impact bounded.
- Helper extraction is a sizeable code move; reviewed via regression tests.

**Locked in until superseded**
- Static unroll depth = `METATABLE_INDEX_MAX_HOPS`.
- Budget-exhaustion falls through to raw write on inner-most reached target.
- Function-form terminates each hop (no chaining through Function callee return).

## Documentation updates

- [x] §8 — adds 0170 (when SoT next refresh).
- [x] ADR 0168 future-work — Multi-hop bullet RESOLVED.

## Test count delta

```
Step 0: 1378 (after ADR 0169)
C2 (2 e2e Red Day 0): 1378 → 1378
C3 (impl): 1378 → 1380
```

## Critical files

- `src/codegen/emit.rs`:
  - NEW helper `emit_number_key_indexassign_routed` containing the existing routing logic + depth recursion.
  - `IndexAssign` Number-key arm collapses to: NaN trap + too_small trap + call helper.
- `tests/phase2_6plus_multi_hop_number_key_newindex.rs` (NEW) — 2 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Existing single-hop Table (0168) / Function (0169) regress | Helper is verbatim extraction; depth-0 path = raw write; depth-N path = identical to before for the leaf hop. Pinned by 0168 / 0169 tests. |
| Code-size explosion at depth 8 | Single call site; modest. |
| Recursion through Function-form callee | Not chained per Lua; Function arm sets handled_by_fn and we stop. |

## Future work

- Mid-array `TAG_NIL` trigger.
- Multi-hop with mixed Table → Function chains in a single statement.
- Runtime depth counter (covers ADR 0134 / 0167 / 0170 deferrals).

## References

- [ADR 0167](0167-multi-hop-number-key-index.md) — read-side multi-hop (mirror).
- [ADR 0168](0168-newindex-number-key-table-form.md) — single-hop Table form (extension source).
- [ADR 0169](0169-newindex-function-form-number-key.md) — single-hop Function form (preserved at every hop).
- Lua 5.4 reference manual §2.4 — `__newindex` semantics.
