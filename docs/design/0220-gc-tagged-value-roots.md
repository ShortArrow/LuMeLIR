# 0220. GC TaggedValue Chunk-Slot Roots + pcall Depth Restore

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third (closing) M3 sub-ADR. [ADR 0218](0218-gc-chunk-safe-real-freeing.md) introduced the chunk-level String-slot root table; [ADR 0219](0219-gc-user-fn-frame-safety.md) added user-fn re-entrancy gating. Both excluded `ValueKind::TaggedValue` chunk locals from the safety predicate. But TaggedValue is the slot kind for multi-assigns from builtins/user-fns (`pcall`, `next`, `getmetatable`), `io.read`, table reads with unknown element kind, and any context where the static kind can be `nil` or a value. Without TaggedValue support the safety predicate rejects a large class of real Lua programs.

This ADR adds TaggedValue scanning via a parallel root table and fixes the pcall + setjmp interaction with `g_gc_fn_depth` (caught longjmp out of `f` would otherwise leave depth incremented, leaking the safety-mode fallback permanently).

## Scope (literal)

- ✅ Two new mutable globals: `g_chunk_tv_table` (i64 ptrtoint of an `[N x ptr]` alloca'd table of TaggedValue slot addresses) and `g_chunk_tv_count` (i64).
- ✅ `chunk_safe_for_real_gc` predicate relaxed to allow `ValueKind::TaggedValue` chunk locals.
- ✅ `emit_main` initialises each TaggedValue chunk slot to Nil-tagged (`emit_value_slot_store_nil`) before the user body runs; collects slot addresses into the TV root table.
- ✅ `emit_gc_mark_from_chunk_roots_real` adds a parallel TV scan after the String scan. For each TV entry: load the 16-byte tagged slot address; read tag at offset 0; when tag == `TAG_STRING`, read payload ptr at offset 8 and compare against the current `g_gc_head` user-payload ptr.
- ✅ Both pcall emit arms (single-return and multi-return) save `g_gc_fn_depth` before the nested `_setjmp` and restore it in both the success (then) and caught (else) branches — preventing depth leak across a longjmp.
- ❌ Tables (`ValueKind::Table`) in chunk slots. Requires DFS through array + hash + element TaggedValues; future ADR.
- ❌ Per-frame TaggedValue tracking inside user fns. ADR 0160 long-term design.
- ❌ Closure cells / upvalue boxes as roots. The Function-Local fast path (ADR 0219) covers non-capturing fns; capturing closures stay disqualifying.
- ❌ `TAG_TABLE` payload following. Tables are not yet supported as chunk slots so no TV entry can carry a TAG_TABLE payload safely.
- ❌ DFS through followed TAG_STRING payload (strings have no internal pointers — none needed).

## Decision

### Predicate relaxation

```rust
chunk.locals.iter().all(|l| matches!(
    l.kind,
    Number | Bool | Nil | String | Function(_) | TaggedValue
))
```

### TV root table init in `emit_main`

```mlir
// for each chunk.locals[i] where kind == TaggedValue:
//   emit_value_slot_store_nil(slots[i])  // tag = TAG_NIL, payload = 0
//   tv_slot_addrs.push(slots[i])
// tv_arr = llvm.alloca i32 N x ptr
// for idx, addr in tv_slot_addrs:
//   store addr, tv_arr + idx * 8
// store ptrtoint(tv_arr), @g_chunk_tv_table
// store N, @g_chunk_tv_count
```

### TV scan shape

```text
for user_iv in g_gc_head:
    found = string_root_scan(user_iv)
    if !found:
        for i in 0..g_chunk_tv_count:
            tv_slot = g_chunk_tv_table[i]
            tag = load(tv_slot)
            if tag == TAG_STRING:
                payload = load(tv_slot + 8) as ptr
                if ptrtoint(payload) == user_iv: found = true; break
    if found: mark BLACK(cur_ptr)
```

### pcall depth save/restore

Before `_setjmp(g_jmpbuf)`:
```rust
let saved_depth = load(g_gc_fn_depth);
```

In both the then-branch (after fn returns + jmpbuf restore) and else-branch (longjmp caught + jmpbuf restore):
```rust
store(saved_depth, g_gc_fn_depth);
```

Idempotent for the success path (fn's matched entry/exit increments leave depth at saved_depth already); load-bearing for the caught path (fn's exit decrement was skipped by longjmp).

## Tests

`tests/phase4_gc_taggedvalue_roots.rs` (NEW, 3 e2e):

1. **TaggedValue err string rooted across collection**: `pcall(bad)` catches a GC-allocated concat string; `print(err)` after `collectgarbage()` still produces the message. Proves TV scanning works.
2. **Caught pcall doesn't leak fn depth**: after a caught error, the post-pcall `collectgarbage()` runs in real-freeing mode (`freed > 0`). Proves the depth restore is load-bearing.
3. **pcall success path preserves real freeing**: after successful pcall + concat chain, intermediates are freed normally.

## Test count delta

```
Step 0:  1491 (after ADR 0219)
C3 (impl + 3 e2e): 1491 → 1494
```

## References

- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — String-slot root table.
- [ADR 0219](0219-gc-user-fn-frame-safety.md) — re-entrancy counter.
- [ADR 0217](0217-pcall-multireturn-abi.md) — pcall multi-return; the canonical TaggedValue producer covered here.
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell layout (out-of-scope for this ADR).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M3 milestone.
