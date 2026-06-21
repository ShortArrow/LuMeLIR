# 0254. GC Table Array-Part Propagation (N2-A)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First N2 (M3-extended) sub-ADR per the [2026-06-21 roadmap rebuild](../notes/roadmap-2026-06-21-rebuild.md). Prior state (ADRs 0218-0220): the chunk-safe predicate excluded `ValueKind::Table` slots — programs with Table-typed Locals stayed in v1 safety mode (mark-all-BLACK, sweep frees nothing) because the GC mark phase had no way to traverse Table children.

This ADR widens the chunk-safe predicate to include Tables AND adds a second mark-phase pass that walks each reachable Table's `array_buf` and marks referenced String objects BLACK. Single-level depth — Tables-in-Tables and the hash part defer to N2-B.

## Scope (literal)

- ✅ `chunk_safe_for_real_gc` widens to allow `ValueKind::Table`. Programs with Table-typed chunk Locals now route through the chunk-roots scan instead of v1 safety mode.
- ✅ Table slot addresses registered into `g_chunk_root_table` (same global as String slots; both store a single user_ptr at offset 0).
- ✅ Every chunk root slot (String AND Table) null-initialised at `main` entry. Without this, an auto-trigger `collectgarbage` firing during Table allocation could read garbage from an uninitialised Table slot.
- ✅ New mark-phase second pass `emit_gc_mark_table_array_propagation`: walks `g_gc_head`; for each obj that is BLACK AND `GC_TYPE_TABLE`, marks its `array_buf` + `hash_buf` BLACK then iterates `array_buf` elements `[0..len)`. For each `TAG_STRING` element, marks the referenced String BLACK via membership-checked write.
- ✅ Membership-checked write (`mark_user_ptr_black_if_nonnull`): before writing the mark byte, walks `g_gc_head` to verify the target is a tracked allocation. Static String literals (ADR 0024 / 0112) live in `.rodata`; without the membership check, marking them would segfault.
- ✅ Hash buf marked BLACK to prevent sweep from freeing it before N2-B implements hash-part walking. Hash-part transitive children NOT yet marked (deferred).
- ❌ Hash-part element walking. N2-B scope.
- ❌ `TAG_TABLE` elements walked recursively. Tables-in-Tables stay WHITE through the propagation — they would be freed if not also reachable as direct chunk roots. N2 fixpoint follow-up.
- ❌ Closure cell + upvalue boxes as GC roots. N2-C scope.
- ❌ Per-frame stack walk inside user fn bodies. N2-D scope (per ADR 0160 design).
- ❌ `TAG_FUNCTION` elements in Table arrays. Function-via-Table reads are runtime-tag-checked downstream; the Function payload's closure cell would need to be marked. Future scope.

## Decision

### Predicate widening + slot registration

```rust
chunk_safe_for_real_gc: locals_ok permits ValueKind::Table

emit_main:
  for each chunk Local of kind String OR Table:
    null-init the slot (emit_store null_ptr, slot)
    push slot's address into g_chunk_root_table
```

The root scan in `emit_gc_mark_from_chunk_roots_real` already reads each entry as a ptr-typed slot content and compares to `g_gc_head` entries' user_ptr. Tables fit unchanged because the slot stores a single ptr (the Table header user_ptr).

### Second pass shape

```text
for each obj in g_gc_head:
    if mark==BLACK AND type_tag==GC_TYPE_TABLE:
        table_ptr = obj.user_ptr
        mark_user_ptr_black_if_nonnull(table_ptr.array_buf)
        mark_user_ptr_black_if_nonnull(table_ptr.hash_buf)
        for i in 0..table_ptr.len:
            slot = table_ptr.array_buf + i*16
            if slot.tag == TAG_STRING:
                mark_user_ptr_black_if_nonnull(slot.payload)
```

### Membership-checked write

`mark_user_ptr_black_if_nonnull(user_ptr)`:

```text
if user_ptr == NULL: return
for each obj in g_gc_head:
    if obj.user_ptr == user_ptr:
        obj.mark = BLACK
        return  (implicit; the loop continues but no further match expected)
```

The inner scan is O(N) per call where N = `g_gc_head` size. Total propagation cost is O(N * M * E) where M = number of BLACK Tables, E = max elements per Table. For typical chunks (a handful of Tables, dozens of elements) this is acceptable. A future optimisation can maintain a pointer hash-set during the walk.

### Why the verify-before-write is load-bearing

Static String literals are emitted as LLVM `.rodata` globals (ADR 0024). Writing to their notional "GC mark byte" — `user_ptr - GC_HEADER_SIZE + GC_HEADER_OFF_MARK` — hits read-only memory and segfaults. Tables can hold mixed static + heap String references; the propagation can't distinguish at element-read time. The membership check is the cleanest filter.

## Tests

`tests/phase4_n2a_table_array_propagation.rs` (NEW, 6 e2e):

1. `local t = {"hello"}; collectgarbage(); print(t[1])` → `"hello"` — Table + String survive.
2. Multi-element Table survives.
3. Transient `tostring(1)` outside the Table is freed (positive `collectgarbage()` delta) while Table contents are preserved.
4. `local t = {}; t[1] = "written"; collectgarbage(); print(t[1])` — runtime-assigned String survives.
5. Reassignment doesn't corrupt; new value reads after collection.
6. Two independent Tables each keep their respective Strings.

## Test count delta

```
Step 0:  1617 (after ADR 0253)
N2-A (impl + 6 e2e): 1617 → 1623
```

## What this unblocks

Per the roadmap rebuild's dependency graph, N2-A is the first of four N2 sub-pieces. Even partial Table tracking opens up:

- Programs with Table-typed Locals + String elements now run with real GC freeing (no longer trapped in v1 safety mode).
- The infrastructure (Table-as-root + second pass) is now available for N3-A (`__gc` dispatch) to hook into.

Still gated until N2-B/C/D land:
- Tables in chunk slots with Tables-as-elements (`{{"nested"}}`) lose the inner Tables on collect.
- Capturing closures with Table upvalues — currently disqualified by the closure non-capturing check.
- Per-fn-frame Tables / Strings (not in chunk slots, allocated inside a user fn) still go through v1 safety mode while inside the fn (per ADR 0219 depth guard).

## References

- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — chunk-safe predicate + chunk root table.
- [ADR 0220](0220-gc-tagged-value-roots.md) — TaggedValue parallel root table + Table-exclusion limitation this ADR lifts.
- [ADR 0185](0185-gc-mark-sweep-v1-safety-mode.md) — v1 mark/sweep skeleton.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string layout the propagation walks.
- [ADR 0024](0024-phase2-7a-string-literal.md) — static String literal globals (the .rodata problem).
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N2-A in the N1-N10 path.
