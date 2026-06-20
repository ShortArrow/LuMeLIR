# 0219. GC User-Fn Frame Safety — Re-entrancy Counter

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M3 sub-ADR. [ADR 0218](0218-gc-chunk-safe-real-freeing.md) landed chunk-level real freeing but excluded any chunk with user functions from the safety predicate, because allocations live in a user-fn frame are not tracked by the chunk root table and would be erroneously freed if the GC mark phase ran mid-execution.

This ADR relaxes the predicate to allow non-capturing user functions by introducing a re-entrancy counter that gates the GC into v1 safety mode whenever any user-fn frame is active. After the fn returns, chunk-level `collectgarbage()` reverts to real freeing.

[ADR 0160](0160-gc-stack-walk.md)'s per-frame slot enumeration via libunwind remains the long-term design for true frame-aware GC; this ADR is the pragmatic interim that closes a much larger class of programs into "real freeing for chunk-level allocations" territory without paying the libunwind integration cost.

## Scope (literal)

- ✅ New global `g_gc_fn_depth: i64` (initialised 0). Incremented at every user `llvm.func` entry, decremented immediately before the trailing `llvm.return`.
- ✅ `chunk_safe_for_real_gc` predicate relaxed: `chunk.functions` may be non-empty as long as every entry has `upvalues.is_empty()` (non-capturing closures only — the closure cell is a static module global, not in `g_gc_head`). `chunk.locals` may include `ValueKind::Function(_)` entries.
- ✅ `emit_gc_mark_from_chunk_roots` reads `g_gc_fn_depth` at runtime. If non-zero, falls back to `emit_gc_mark_inline` (mark-all-BLACK v1 safety mode). If zero, runs the ADR 0218 root scan.
- ✅ Single increment + single decrement site per user fn (LuMeLIR fns route every body return through one trailing `emit_llvm_return` at the bottom of `emit_function`).
- ❌ Capturing closures. The cell is heap-allocated and would be freed if not in the root table; future ADR adds the cell as a root.
- ❌ Tables / TaggedValue locals. Predicate still rejects them.
- ❌ Per-frame slot enumeration per ADR 0160. The depth counter is a pragmatic short-circuit, not a replacement for the stack-walk design.

## Decision

### Re-entrancy counter

```rust
// emit_function entry:
let depth = load(g_gc_fn_depth);
store(depth + 1, g_gc_fn_depth);

// emit_function trailing return:
let depth = load(g_gc_fn_depth);
store(depth - 1, g_gc_fn_depth);
emit_llvm_return(...);
```

The counter is a simple thread-unsafe i64. Coroutines (future) will need a thread-local equivalent; for the current single-threaded compiled output the global is sound.

### Mark phase dispatch

```rust
if g_gc_fn_depth != 0 {
    // Inside a user fn — fall back to v1 safety mode.
    emit_gc_mark_inline()
} else {
    emit_gc_mark_from_chunk_roots_real()
}
```

Auto-trigger from `emit_gc_alloc` inside a user fn still fires mark + sweep; mark paints everything BLACK so sweep frees nothing. After the fn returns, depth drops to 0 and the next collection reverts to real freeing.

## Tests

`tests/phase4_gc_with_user_fns.rs` (NEW, 3 e2e):

1. Chunk-level collection after a user fn returns observes `freed > 0` for transient strings produced by the fn.
2. `collectgarbage()` called from inside a user fn returns 0 (v1 safety mode active).
3. A string returned by a user fn is rooted in its chunk slot and survives a chunk-level collection.

## Test count delta

```
Step 0:  1488 (after ADR 0218)
C3 (impl + 3 e2e): 1488 → 1491
```

## References

- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — chunk-safe foundation.
- [ADR 0160](0160-gc-stack-walk.md) — long-term per-frame stack walk design; this ADR is the pragmatic interim.
- [ADR 0083](0083-phase2-5c-full-closures.md) — non-capturing closure singleton globals (proves the cell is not in `g_gc_head`).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M3 milestone.
