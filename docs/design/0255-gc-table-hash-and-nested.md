# 0255. GC Hash-Bucket Walk + Tables-in-Tables (N2-B)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second N2 sub-ADR. [ADR 0254](0254-gc-table-array-propagation.md) added Table array-part walk for single-level depth. Real Lua programs use hash-keyed Table fields (`t.name = "world"`) and nested Tables (`{ {"inner"} }`). Both shapes were excluded from N2-A: hash buckets weren't walked at all, and TAG_TABLE elements in array slots were ignored (they would be freed across collection).

This ADR closes both gaps:

1. **Hash-bucket walk** — for each BLACK Table with a non-null `hash_buf`, iterate buckets `[0..hash_cap)` and mark TAG_STRING / TAG_TABLE references in both the key and value tagged slots.
2. **Tables-in-Tables** — array slots with TAG_TABLE now propagate their references. Hash slots already join the same path via item 1.
3. **Fixed-iteration outer loop** — the propagation pass runs 8 times so deeply-nested Tables (up to 8 levels) get fully marked. Each iteration's effect is monotonic (marks only get added), so the bound is a correctness ceiling.

## Scope (literal)

- ✅ Hash-bucket walk added to `emit_gc_mark_table_propagation_pass`. Reads `hash_cap` from `HASH_OFF_CAP=0`; iterates `hash_cap` buckets at offset `HASH_OFF_ENTRIES=16`; each bucket is `HASH_ENTRY_SIZE=32` bytes (16-byte key tagged slot + 16-byte value tagged slot).
- ✅ For both key and value slots: read tag at slot offset 0; if `TAG_STRING` or `TAG_TABLE`, mark the user_ptr at slot offset 8 BLACK via the existing membership-checked helper.
- ✅ TAG_TABLE handling in array slots: extends the N2-A `is_string` check to `is_string OR is_table_elem`.
- ✅ Fixed-iteration outer loop: `emit_gc_mark_table_array_propagation` calls the propagation pass 8 times. Each iteration marks any newly-reachable objects; 8 levels of nesting is well past typical Lua usage.
- ❌ True fixpoint with "did mark" flag. The fixed iteration ceiling is simpler in MLIR — threading an i1 changed flag through 5 nested scf regions is verbose. The 8-iteration ceiling is a documented limitation.
- ❌ Empty / deleted bucket skip optimisation. The walk currently visits every bucket including TAG_NIL / TAG_DELETED ones; the membership-check guard makes them no-ops but the iteration cost remains O(hash_cap) per pass.
- ❌ TAG_FUNCTION element walking. Closure cells in Table values would need their own propagation; deferred to N2-C alongside chunk-Function root expansion.
- ❌ Static Lua String literals as hash keys. Static literals (ADR 0024) aren't in `g_gc_head`; the membership check no-ops them, which is correct — they don't need marking.

## Decision

### Hash-bucket layout reuse

The hash table layout (ADRs 0058 / 0079) puts a 16-byte header (`cap`, `count`) then `cap` × 32-byte entries. Each entry is a key tagged slot at offset 0 + value tagged slot at offset 16. The propagation walks the same layout sans tombstone awareness — `TAG_DELETED` / `TAG_NIL` slots have no payload ptr to mark, and the membership check filters them.

### Why fixed 8 iterations

Threading a "did mark" flag through the nested scf structure (outer g_gc_head walk → BLACK-Table branch → array walk loop → element scf.if → inner mark) requires updating 5+ scf regions to carry the i1. Each loop's `before` / `after` regions need the carrier in their block arg lists. The MLIR cost is high; the implementation cost is correctness-prone.

Fixed iteration trades one well-defined ceiling for code clarity. 8 levels of Tables-in-Tables is well past anything typical Lua code constructs (3 levels is already unusual). The ceiling is documented; future ADRs can swap to a true fixpoint if a real workload exceeds it.

### Composition with N2-A's mark helper

`mark_user_ptr_black_if_nonnull` is reused unchanged. It walks `g_gc_head` once per call to verify membership before writing the mark byte; the per-mark cost is O(N) where N = `g_gc_head` size. Across 8 iterations × M reachable Tables × E elements per Table × N g_gc_head entries, the total cost is O(8 × M × E × N). For small chunks (M < 10, E < 100, N < 1000) this is < 8M operations per `collectgarbage()` call — fast enough.

## Tests

`tests/phase4_n2b_table_hash_and_nested.rs` (NEW, 7 e2e):

1. `t = {}; t.name = "world"; collectgarbage(); print(t.name)` → `"world"`.
2. Multi-key hash (3 string-keyed entries) survives.
3. Nested Table in array slot survives (`{inner}` then `print(outer[1][1])`).
4. Nested Table in hash slot survives (`t.inner = payload`).
5. Three-level nest (`outer[1][1][1]`) survives — proves the fixed-iteration outer loop covers transitive reachability.
6. Mixed array + hash (`{"first","second"}; t.label="my-table"`) — both parts walked.
7. Transient String outside the kept tree is freed (`tostring(7)` → `freed > 0`).

## Test count delta

```
Step 0:  1623 (after ADR 0254)
N2-B (impl + 7 e2e): 1623 → 1630
```

## What this unblocks

N2-B + N2-A together cover the most common Lua data shapes:

- Tables as Lua's primary data structure: array, hash, mixed, nested — all GC-tracked.
- Programs using object-like patterns (`local t = {name="x", items={"a","b"}}`) now compile + run with real freeing instead of v1 safety mode.
- The infrastructure for N3-A (`__gc` finalizer dispatch) can now hook into the second pass — when a BLACK Table is about to be swept-WHITE-and-freed, its metatable's `__gc` can be probed.

Still gated until N2-C / N2-D land:
- Capturing closures with Table upvalues — currently rejected by the chunk-safe predicate's non-capturing check.
- Tables allocated inside user fn bodies (not in chunk slots) — still go through v1 safety mode while inside the fn (ADR 0219 depth guard).

## References

- [ADR 0254](0254-gc-table-array-propagation.md) — N2-A array-part foundation.
- [ADR 0058](0058-phase2-6b-hash.md) — hash_buf layout.
- [ADR 0079](0079-phase2-6b-hash-keys.md) — hash entry tagged-slot widening.
- [ADR 0184](0184-gc-type-meta-size-guard.md) — 4 GiB cap that bounds hash_cap.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N2-B in the N1-N10 path.
