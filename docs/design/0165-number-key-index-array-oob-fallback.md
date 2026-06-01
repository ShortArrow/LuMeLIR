# 0165. Number-Key Array OOB `__index` Table-Form Fallback

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

ADR 0134 test 7 (`array_oob_falls_through_to_metatable_index`) was the last deferred Red on the Phase 2.6+ corpus. The test expects:

```lua
local fallback = {}
fallback[5] = 555
local t = {1, 2, 3}
local mt = {}
mt.__index = fallback
setmetatable(t, mt)
print(t[5])  -- expected: 555
```

Today (pre-fix): `t[5]` writes Nil to the tagged dst slot via the existing OOB path in `emit_local_init_tagged` (Number-key arm) and does NOT consult the metatable chain. The hash-eligible-key path already had `emit_metatable_index_fallback_if_nil` wired (ADR 0134), but the Number-key path was deferred because it required either an ABI shift (`f64` direct return → TaggedValue widening) or a parallel fallback implementation.

This ADR is the **parallel fallback** approach — single-hop Table-form `__index` only. Multi-hop chains and Function-form Number-key `__index` (e.g. `mt.__index = function(t, k) ... end` with Number k) remain deferred.

## Decision

### Scope (literal)

- ✅ Single-hop Number-key `__index` Table-form fallback at the `emit_local_init_tagged` Number-key OOB path.
- ✅ Lookup of `__index_table[key_i]` via the array path (matches the existing array-OOB semantics).
- ❌ Multi-hop chains (`__index = chain of tables`).
- ❌ Function-form Number-key `__index`.
- ❌ Number-key Index against String / Bool / Function `__index` payloads (still no-op Nil).
- ❌ The flat-Number-Index codegen path at `emit_expr` (`HirExprKind::Index` arm with Number key directly returning f64). That path **traps** on OOB; the fallback applies only to TaggedValue-widening consumers like `print(t[5])` / `local x = t[5]` (TaggedValue local) / `tostring(t[5])`.

### Implementation

New helper `emit_number_key_metatable_index_fallback(target_ptr, key_i, dst_slot, ...)` in `src/codegen/emit.rs`. Called from the OOB `then_blk` of `emit_local_init_tagged`'s Number-key arm, **after** `emit_value_slot_store_nil(dst_slot)`.

Algorithm:

1. Load `mt_ptr = *(target + TABLE_OFF_METATABLE)`.
2. If `mt_ptr` is null → no-op (dst stays Nil).
3. Probe `mt["__index"]` via `emit_hash_lookup_into_tagged_slot(NilOnMissing)` into a tmp tagged slot.
4. Load the probe tag. If `TAG_TABLE`, extract `__index_table` ptr from payload.
5. Bounds-check `__index_table[key_i]`: in-range → raw 16-byte slot copy into `dst_slot`; OOB → leave Nil.
6. All other probe tags (Nil / Function / Number / etc.) → leave Nil.

## Alternatives considered

- **ABI-shift Number-key Index → TaggedValue widening**. Rejected — would require migrating every `emit_expr` `HirExprKind::Index` Number-key caller (consumer count is significant; Phase 2.6+ was scoped to NOT touch this).
- **Recurse through `emit_check_metatable_index_with_depth`**. Rejected — that helper assumes hash-key lookup of the recursed table. For Number-key, we need the array path; adapting the helper would diverge its contract.
- **Make Number-key Index always trap on OOB**. Rejected — `print(t[5])` returning Nil is the documented expected behaviour pre-fallback; trapping would break tests.

## Consequences

**Positive**
- ADR 0134 test 7 (last deferred Red) goes Green.
- Phase 2.6+ corpus is now fully Green (0 deferred reds).
- Helper is self-contained — no signature changes ripple through emit.rs.

**Negative**
- Single-hop only — chained Table `__index` for Number-keys still misses.
- Function-form Number-key `__index` still misses.
- The OOB hot path now does additional metatable work even when no `__index` is set (single null-check costs 1 load + 1 cmpi; negligible).

**Locked in until superseded**
- Single-hop scope. Multi-hop / Function-form Number-key `__index` are explicit future-work bullets.

## Documentation updates

- [x] §4 LIC — new `LIC-number-key-index-array-oob-fallback-1`.
- [x] §8 — adds 0165.
- [x] ADR 0134 future-work — test 7 marked RESOLVED by this ADR.

## Test count delta

```
Step 0:   1370 (after ADR 0164)
C1 (impl in this commit): 1370 → 1370 (test 7 flips from Red to Green; count unchanged)
```

Now: **0 deferred reds. Full corpus Green.**

## Critical files

- `src/codegen/emit.rs`:
  - New helper `emit_number_key_metatable_index_fallback` (~150 LOC).
  - `emit_local_init_tagged` Number-key OOB `then_blk` calls the helper after `emit_value_slot_store_nil`.

## Risks

| Risk | Mitigation |
|---|---|
| Existing Number-key OOB behaviour regresses | The fallback only runs when (a) OOB AND (b) mt is non-null AND (c) `__index` is Table. All other paths unchanged. |
| Multi-hop chains silently leak to Nil | Documented out-of-scope. Future ADR can recurse. |
| Flat-f64 Number-key Index at `emit_expr` still traps | Documented; the flat path only fires in contexts that statically expect Number (e.g. `local x: Number = t[5]`); rare in practice. Future ADR can widen to TaggedValue. |

## Future work

- Multi-hop Number-key `__index` chain.
- Function-form Number-key `__index`.
- Flat-f64 Number-key Index OOB widening (optional; only if a real workload demands).
- IndexAssign analog for Number-key `__newindex` (ADR 0135 deferral pair).

## References

- [ADR 0134](0134-metatables-index-read.md) — `__index` Table form (test 7 lived here).
- [ADR 0088](0088-phase2-6b-hash-lookup-miss.md) — `emit_hash_lookup_into_tagged_slot` reused for the `__index` field probe.
- [ADR 0150](0150-index-function-form.md) — Function form `__index` for static-String key (Number-key Function form deferred).
- Lua 5.4 reference manual §3.4.10 — `__index` semantics.
