# 0218. GC Chunk-Safe Real Freeing — String-Slot Root Table

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M3 sub-ADR. [ADR 0185](0185-gc-mark-sweep-v1-safety-mode.md) landed mark + sweep functions in v1 safety mode — every `g_gc_head` entry was pre-marked BLACK so sweep freed nothing. The v1 contract was the principled bridge that let collectgarbage() wire end-to-end before [ADR 0160](0160-gc-stack-walk.md)'s stack walk produced real roots.

ADR 0160 chose libunwind (or setjmp fallback) for per-frame slot enumeration. That design is correct for the general case (user functions, nested frames) but the implementation is multi-session work: per-frame slot tables, caller-side save/restore, libunwind feature gate. M3 needs a faster path to demonstrably-freeing GC.

This ADR lands a narrow, demonstrably-correct first cut: **chunk-level String-slot root table**. For programs that satisfy a static safety predicate (no Tables, user functions, or TaggedValue locals), the GC mark phase scans only main's String-kind slot addresses. Transient string allocations not anchored to any chunk slot become WHITE and sweep frees them. Programs that don't satisfy the predicate stay in v1 safety mode unchanged.

## Scope (literal)

- ✅ Static `chunk_safe_for_real_gc(chunk: &HirChunk) -> bool` predicate. Returns true iff `chunk.functions.is_empty()` AND every `chunk.locals[i].kind` is in `{Number, Bool, Nil, String}`.
- ✅ Two new mutable globals: `g_chunk_root_table` (i64 holding the ptrtoint of the alloca'd root array), `g_chunk_root_count` (i64).
- ✅ `emit_main` zero-initialises every String-kind chunk slot, builds an `[N x ptr]` alloca containing each String slot's address, and writes its ptrtoint + count to the two globals. Done only when `chunk_safe_for_real_gc(chunk)` is true.
- ✅ `register_gc_runtime_funcs` accepts the `chunk_safe` flag. When true, emits `emit_gc_mark_from_chunk_roots` instead of the v1 safety-mode walker.
- ✅ `emit_gc_mark_from_chunk_roots` walks `g_gc_head` once; for each tracked object computes its user-visible payload ptr (`raw + GC_HEADER_SIZE`), then linearly scans the chunk root table for a slot whose contents equal that payload ptr. On match, sets the header mark byte to BLACK. Roots pointing at static string literals (which are not in `g_gc_head`) never match and are safely ignored — preventing mark writes from hitting .rodata.
- ✅ Sweep unchanged from ADR 0185.
- ❌ Per-frame slot tables for user fn frames. Future ADR (ADR 0160 implementation).
- ❌ DFS through Table array + hash parts. Future ADR — Tables in chunk slots disqualify the chunk-safe gate.
- ❌ TaggedValue locals (runtime tag dispatch for ptr-carrying tags). Future ADR.
- ❌ Closure cells (function values) as roots. Capturing closures get freed if reached only through chunk slots; non-capturing fns are static globals and were never tracked.
- ❌ Mixed-chunk strategy. A chunk either passes the safety predicate (real freeing) or falls back to v1 safety mode wholesale. No per-region grain.

## Decision

### Static safety predicate

```rust
pub(crate) fn chunk_safe_for_real_gc(chunk: &HirChunk) -> bool {
    chunk.functions.is_empty()
        && chunk.locals.iter().all(|l| matches!(
            l.kind,
            ValueKind::Number | ValueKind::Bool | ValueKind::Nil | ValueKind::String
        ))
}
```

A chunk that defines no user functions and has only scalar / String slots is "safe" in the sense that its mark phase need only consider String pointers — no transitive structure to traverse. This excludes the majority of real Lua programs but precisely matches the demo workload for proving the GC freeing path is wired.

### Root table layout

At the top of `main`, after slot allocas and the ADR 0215 setjmp pad:

```mlir
// for each chunk.locals[i] where kind == String:
//   llvm.store null_ptr, slots[i]   // skip uninitialised garbage
// root_arr = llvm.alloca i32 N x ptr
// for idx, slot_addr in string_slot_addrs:
//   entry_ptr = root_arr + idx * 8
//   llvm.store slot_addr, entry_ptr
// llvm.store ptrtoint(root_arr), @g_chunk_root_table
// llvm.store N, @g_chunk_root_count
```

The alloca lives for the duration of `main`, so its address remains valid throughout the chunk body. The mark phase reads `g_chunk_root_table` + `g_chunk_root_count` at every `collectgarbage()` call.

### Mark phase shape

```text
for cur_iv in g_gc_head:
    cur_ptr  = inttoptr(cur_iv)
    user_ptr = cur_ptr + GC_HEADER_SIZE
    found    = false
    for i in 0..g_chunk_root_count:
        slot_addr = g_chunk_root_table[i]
        slot_val  = load(slot_addr)
        if ptrtoint(slot_val) == ptrtoint(user_ptr): found = true; break
    if found: store(BLACK, cur_ptr + GC_HEADER_OFF_MARK)
```

Inverting the loop (walk GC chain, scan roots per object) avoids the .rodata problem: static string literal pointers never appear in `g_gc_head`, so a slot holding a literal pointer never matches any tracked object and the mark phase never writes to the literal's memory.

## Tests

`tests/phase4_gc_real_freeing.rs` (NEW, 3 e2e):

1. Rooted string survives collection — `local s = "hello"; collectgarbage(); print(s)` → `"hello"`.
2. Concat chain intermediates are freed — `local s = "a"; s=s..\"b\"; s=s..\"c\"; s=s..\"d\"; freed=collectgarbage()` → `s = "abcd"` AND `freed > 0`.
3. Second consecutive collect has smaller-or-equal delta — `first >= second`.

`tests/phase3_gc_mark_sweep_v1.rs` (UPDATED, 3 e2e): renamed expectations from v1-safety-mode pins to real-freeing pins. `collectgarbage()` now returns positive delta; `collectgarbage("count")` decreases across a collection with transients; sequential calls keep the heap consistent.

`tests/phase3_gc_auto_trigger.rs` (UPDATED, 1 e2e): the explicit-post-loop case now asserts `freed > 0` (transient `string.rep` results freed) instead of the v1-mode `freed == 0`.

## Test count delta

```
Step 0:  1485 (after ADR 0217)
C3 (impl + 3 new e2e + 4 updated): 1485 → 1488
```

## References

- [ADR 0185](0185-gc-mark-sweep-v1-safety-mode.md) — v1 safety mode; superseded for chunk-safe programs.
- [ADR 0186](0186-gc-auto-trigger-and-func-factoring.md) — auto-trigger threshold; now meaningfully fires.
- [ADR 0160](0160-gc-stack-walk.md) — full per-frame stack walk strategy; this ADR is the chunk-only subset.
- [ADR 0157](0157-phase3-gc-allocator-wrapper.md) — `emit_gc_alloc` chokepoint that registers every allocation into `g_gc_head`.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M3 milestone.
