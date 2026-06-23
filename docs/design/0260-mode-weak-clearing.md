# 0260. `__mode` Weak-Table Pre-Sweep Clearing (N3-B)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Third N3 sub-ADR. ADR 0239 pinned `__mode` as a metatable field that round-trips through Table machinery; the runtime weak-clearing was deferred. This ADR lands the **pre-sweep weak-clear pass** infrastructure: a g_gc_head walk that, for each `GC_TYPE_TABLE` whose metatable carries a non-Nil `__mode`, scans the table's hash buckets and nulls out any value-slot whose `TAG_TABLE` payload references a WHITE GC node.

## Scope (split: N3-B-1 + N3-B-2)

### N3-B-1 (this commit)

- ✅ New module-level constant string `s_mode_field_name = "__mode\0"`.
- ✅ `emit_gc_weak_clear_pass` — runs at the entry of `@gc_sweep`, **before** the existing WHITE-free walk. Iterates `g_gc_head`; for each Table node, probes the metatable for `__mode`; if non-Nil, calls `emit_weak_clear_hash_buckets` on the table.
- ✅ `emit_weak_clear_hash_buckets` — iterates the table's hash bucket entries; for each entry whose value tag is `TAG_TABLE`, reads the payload ptr, locates the referent's GC header (`payload - GC_HEADER_SIZE`), reads the mark byte, and if `GC_MARK_WHITE` writes `TAG_NIL` to the value slot via `emit_value_slot_store_nil`.
- ✅ Skip semantics: non-Table GC nodes, Tables without metatable, metatables with Nil `__mode`, Tables with empty `hash_buf`, and entries whose value isn't `TAG_TABLE` all skip silently.
- ✅ Non-regression: the new pass does NOT clear entries pointing to BLACK objects. Verified by `weak_table_with_rooted_value_does_not_clear` e2e (the value is held by another root, stays BLACK, weak-clear leaves it).
- ✅ Strong tables (no `__mode`) are unaffected. Verified by `strong_table_keeps_entry_through_gc`.

### N3-B-2 (now landed alongside N3-B-1)

- ✅ **Mark-phase `__mode` skip.** Inside `emit_gc_mark_table_propagation_pass`'s `then_blk` (the BLACK-Table-propagation arm), after marking the `array_buf` and `hash_buf` buffers themselves BLACK (so sweep doesn't free them), the pass computes a `not_weak` flag by probing `mt["__mode"]` and stashes it in a 1-byte alloca. The inner mark calls inside the array entry walk and the hash bucket walk gate on a load of that alloca: when `not_weak == 0` the `mark_user_ptr_black_if_nonnull` call is skipped, leaving referenced values WHITE for the pre-sweep weak-clear pass. Cost: one extra hash probe per BLACK Table per propagation iteration (ADR 0255 fixpoint loop wraps 8 passes).
- ❌ **Spec-precise "k" / "v" / "kv" discrimination.** Today's simplification: any non-Nil `__mode` triggers weak-value clearing. Spec wants the string contents to drive whether keys, values, or both are weak. Refining requires a runtime string-first-char inspection (`strchr` or inline cmp on string ptr `[0]`); deferred.
- ❌ **Array-part weak clearing.** Today only the hash part is scanned. The array part is contiguous tagged slots and would follow the same pattern; deferred for code volume.
- ❌ **Weak keys** (`__mode = "k"`). The hash-entry KEY slot's referent isn't checked; only VALUE slots are. Mirror logic; deferred.
- ❌ **Ephemeron tables** (Lua 5.4 §2.5.4 §3rd paragraph). Out of scope.

## Decision

### Why pre-sweep (not interleaved with the WHITE-free loop)

If weak-clearing and freeing run in the same pass, a Table A pointing weakly at table B could be processed before B's free. After A's entries are cleared, B then gets freed. But if B were processed first (freed) before A's clear, A would temporarily hold a dangling ptr in its hash slot until A's iteration freed it. To avoid this windowed dangle, weak-clear runs as a complete first pass; sweep's WHITE-free loop runs after, by which time all weak slots referencing WHITE objects are already cleared.

### Why "any non-Nil `__mode` is weak-v" is OK for now

The canonical user idiom is `setmetatable(cache, {__mode = "v"})` — value-weak caches. A user passing `"k"` likely wants weak-key semantics (which today's pass under-implements: it clears the value slot instead of the entry). This is observable when both keys and values are referenced GC objects. The simplification matches Lua's most common case and matches what most user code observes; spec-perfection is N3-B-2 scope.

### Why mark-phase skip is a separate ADR

The mark propagation passes (ADR 0254 / 0255 / 0257) are deeply nested scf.while / scf.if structures. Wrapping them with an `is_weak_source` predicate requires probing `mt["__mode"]` for every BLACK Table, on every fixpoint iteration (ADR 0255 wraps in 8 iterations). The probe itself does a hash lookup; doing it inside the propagation pass means each fixpoint iteration pays the lookup cost. Refining the cost model and the scf.if shape is enough surface to warrant its own ADR with proper alternative comparison.

## Tests

`tests/phase4_n3b_mode_weak_clear.rs` (NEW, 3 e2e, all Green):

1. **Weak table with rooted value does not clear** — `kept` rooted at chunk level, `cache.handle = kept` with `cache.__mode = "v"`. Mark phase marks `kept` BLACK via chunk root; the value is also reachable from a strong chunk slot so it stays BLACK; weak-clear sees BLACK, leaves it.
2. **Strong table keeps entry through GC** — same shape without `__mode`. The propagation pass marks values BLACK because `not_weak == true`.
3. **Weak value table clears unreachable entry** — only-reference case. `not_weak == false` skips the mark, value stays WHITE, weak-clear nulls the value slot.

`tests/phase4_m10_mode_field_pin.rs` (the ADR 0239 pin tests) stays Green — the field-handling surface is unchanged.

## Test count delta

```
Step 0:  1649 (after ADR 0259)
N3-B-1 (weak-clear pass + 2 new Green e2e + 1 ignored):  1649 → 1651
N3-B-2 (mark-phase __mode skip lifts ignore):  1 ignored → Green;  1651 → 1652
N3-C (Builtin::Newproxy + userdata tag arm) further pushed total to 1655.
```

## What this unblocks

- The pre-sweep weak-clearing infrastructure is in place; N3-B-2 is a focused follow-up on mark propagation only.
- Future ephemeron-table support reuses the same hash-bucket walker.

Still gated:
- Observable weak-value clearing (N3-B-2).
- Spec-precise "k"/"v"/"kv" discrimination.
- Array-part weak clearing.
- Weak-key semantics.

## References

- [ADR 0239](0239-gc-mode-field-pin.md) — predecessor pin; runtime deferral now partially lifted.
- [ADR 0156](0156-gc-architecture-v1.md) — GC architecture roadmap; weak tables in step 8.
- [ADR 0254](0254-gc-table-array-propagation.md) — array-part propagation that N3-B-2 must guard.
- [ADR 0255](0255-gc-table-hash-and-nested.md) — hash-part propagation that N3-B-2 must guard.
- [ADR 0088](0088-table-hash-lookup-chokepoint.md) — hash-lookup chokepoint reused for the `__mode` probe.
- [Lua 5.4 §2.5.4](https://www.lua.org/manual/5.4/manual.html#2.5.4) — garbage collection / weak tables spec.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N3-B in the N1-N10 path.
