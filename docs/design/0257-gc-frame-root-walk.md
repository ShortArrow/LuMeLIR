# 0257. GC Per-Frame Stack Walk (N2-D)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-22
- **Deciders:** ShortArrow

## Context

Fourth N2 sub-ADR — closes the structural gap N2-A / N2-B / N2-C left open. ADR 0219 introduced `g_gc_fn_depth` so a `collectgarbage()` fired inside a user fn body would fall back to v1 safety mode (mark every `g_gc_head` node BLACK). That kept inside-fn allocations live but forfeited any real freeing while the depth counter was non-zero — every transient created in a user-fn body lingered until chunk-level GC could run.

N2-D drops the depth-guard fallback for **frame-safe** chunks and replaces it with a per-frame root chain: each user-fn entry pushes a small linked-list node that exposes the fn's `String` / `Table` / non-param `Function`-kind body locals as roots; the mark phase walks the chunk roots + the frame chain, then runs the existing N2-A/B/C propagation passes.

## Scope (literal)

- ✅ New global `g_frame_root_head: i64` (init 0) — head of the linked frame-root chain.
- ✅ New predicate `frame_safe_for_real_gc(chunk)` — returns true iff `chunk_safe_for_real_gc(chunk)` AND every user-fn's locals are all in `{Number, Bool, Nil, String, Table, Function(_)}`.
- ✅ When frame-safe, every user-fn entry allocates a 24-byte frame node `[next_iv, count, arr_iv]` plus a `count`-entry ptr-array of its String / Table / non-param Function slot ptrs; null-inits those slots; links the node into `g_frame_root_head`.
- ✅ Matching pop on fn exit: load node[0] → store into `g_frame_root_head`.
- ✅ pcall sites save the head pre-setjmp and restore in both then / else regions (alongside the existing ADR 0220 depth save / restore). Without this a longjmp would leave `g_frame_root_head` pointing at a node whose stack frame is gone.
- ✅ When frame-safe, `gc_mark` skips the depth-guard dispatcher and calls `_real` directly. `_real` walks chunk roots + invokes the new `emit_gc_mark_from_frame_chain` between the chunk-root scan and the existing Table / closure-cell propagation.
- ✅ Function-kind PARAMS (idx < params_len) are excluded from the frame root array. Their slot is a block-arg alias (not addressable). They remain rooted via the caller's chunk slot or caller's frame (recursive guarantee).
- ❌ **TaggedValue locals in user fns.** A TV local can hold a `String` / `Table` ptr at offset 8; without per-tag dispatch in the frame walk we'd mis-treat packed Numbers. Parallel TV frame chain is N2-D-2 scope. Until then, any chunk with a user fn that owns a TV-kind local stays in v1 safety mode — `frame_safe_for_real_gc` returns false.
- ❌ Upvalue-box CONTENTS marking (per ADR 0256 §Scope) remains deferred — N2-D adds nothing new there.
- ❌ Cycle detection. Frame walk is acyclic (linked list), but the propagation passes' fixed-iteration fixpoint (ADR 0255) still bounds reachability at depth 8.

## Decision

### Why drop the depth-guard fallback

The v1 fallback was correct (mark-all-BLACK → nothing freed → safe) but pessimistic: long-running user code that allocates Strings or Tables inside a fn would accumulate dead transients until the fn returned and chunk-level GC fired. For programs that compute primarily inside fns (the common case for any non-toy Lua program), the v1 fallback effectively disabled freeing.

The frame chain gives the mark phase the same information the chunk root walk has — addresses of every live root slot — without paying the cost of a global heap scan or per-allocation registration. Per-fn-call overhead is one alloca + one prev/store/store sequence at entry, one load + store at exit. Frame walk cost during `collectgarbage` is `O(depth × slots_per_frame)` slot scans + the existing `O(g_gc_head)` membership check per slot via the shared `mark_user_ptr_black_if_nonnull` helper (ADR 0254).

### Frame node layout

```
node[0]  : i64   next_iv      (prev head, restored on pop)
node[8]  : i64   count        (number of slot ptrs in arr)
node[16] : i64   arr_iv       (ptr-as-i64 to the slot-ptr array)
```

`arr_iv` indexes a separately-alloca'd `[count × ptr]` array — each entry is the address of a fn-local slot (alloca for String / Table / non-param Function). The mark phase iterates the array, loads each slot's current value (the user_ptr the user code wrote), and routes through `mark_user_ptr_black_if_nonnull` — same helper N2-A/B/C use, so the g_gc_head membership check transparently handles static literals and null-init no-ops.

### Why slot ptrs (not values) in the array

Storing slot ADDRESSES (not the slot's current value) means the mark phase reads the LIVE value at GC time — no synchronization needed when the user code reassigns the slot mid-fn. The chunk root table uses the same pattern (ADR 0218).

### pcall interaction

A longjmp out of the called fn unwinds its alloca'd frame node — but `g_frame_root_head` still points at it. Reading the dead-stack memory in the next GC would surface garbage as a slot count or next-ptr. The save/restore mirrors the existing ADR 0220 depth save/restore: snapshot the head before `_setjmp`, write it back in both the success-path and the error-path of the `scf.if` that follows. Two pcall sites (multi-return at emit.rs:~6100 and single-return at emit.rs:~11135).

### Why TV locals stay v1

TaggedValue locals are 16-byte two-i64 slots — tag at offset 0, payload at offset 8. The tag dispatches whether the payload is a packed Number (no marking) or a `String` / `Table` ptr (must mark). To handle TV-kind frame slots correctly the mark phase needs a parallel walker that loads the tag first, then conditionally calls `mark_user_ptr_black_if_nonnull` on the payload. That is N2-D-2 scope. Until then, any chunk where a user fn owns a TV-kind local fails `frame_safe_for_real_gc` and keeps the v1 fallback.

## Tests

`tests/phase4_n2d_frame_root_walk.rs` (NEW, 6 e2e):

1. Fn-local Table survives mid-fn `collectgarbage()` (`{"alive"}` → still readable).
2. Fn-local String survives mid-fn collection (`tostring(42)`).
3. Two fn-local Tables both survive.
4. Mid-fn `collectgarbage()` REALLY frees in-fn transients (`s = tostring(7); s = tostring(8); freed > 0`) — the N2-D differentiator; under v1 fallback this Red'd with `freed == 0`.
5. Nested call (outer's frame stays live while inner runs gc).
6. Transient-after-return freed at chunk level.

`tests/phase4_gc_with_user_fns.rs`: the obsolete `collect_inside_user_fn_falls_back_to_v1_safety_mode` test (which encoded the ADR 0219 invariant) is replaced with `collect_inside_user_fn_does_real_freeing_via_frame_walk` — same shape, opposite expected outcome.

## Test count delta

```
Step 0: 1635 (after ADR 0256)
N2-D (impl + 6 new e2e + 1 inverted test): 1635 → 1641
```

## What this unblocks

- Real freeing of inside-fn transients during long-running user code. Mid-fn `collectgarbage()` is now functional, not a no-op.
- `__close` (ADR 0153 deferred) scope-exit hook can rely on real GC running mid-fn.
- N3 (per-slot type metadata) work — the frame chain shape generalises to TV slots once tag dispatch is added.

Still gated:
- TV locals in user fns (TV frame chain — N2-D-2).
- Upvalue-box CONTENTS marking (per ADR 0256 §Scope).

## References

- [ADR 0219](0219-gc-fn-depth-guard.md) — predecessor depth-guard that N2-D supersedes for frame-safe chunks.
- [ADR 0220](0220-gc-pcall-depth-restore.md) — depth save/restore precedent; N2-D adds head save/restore alongside.
- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — chunk root table layout (frame node mirrors it).
- [ADR 0254](0254-gc-table-array-propagation.md) — `mark_user_ptr_black_if_nonnull` membership helper reused.
- [ADR 0255](0255-gc-table-hash-and-nested.md) — fixpoint propagation that runs after the frame walk.
- [ADR 0256](0256-gc-capturing-closures.md) — sibling N2-C; closure cells in fn-local Function slots now stay live via the frame walk.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N2-D in the N1-N10 path.
