# 0160. Phase 3 GC step 4 — Stack Walk for Alloca Roots

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

ADR 0156 deferred the stack walk for local alloca slots to this ADR. Without it, `collectgarbage()` can't see local Lua objects and would either free everything (ADR 0159 v1 safety mode) or do nothing useful. This ADR commits to a strategy.

Decision-only. Implementation lands in a follow-up commit.

## Decision

**Use libunwind for per-frame slot enumeration**, falling back to libc `setjmp` when libunwind is unavailable.

### Approach: per-frame slot registry

Each user function emits, at function entry, a write to a thread-local "current frame slot table" describing the layout of its alloca slots:

```
g_current_frame_slots: thread_local ptr  // points into the active function's stack frame
g_frame_slot_count:    thread_local i64  // number of slots in the active frame
```

At each call site, the caller saves+restores these globals around the callee call. At `collectgarbage()` time, the mark phase walks via libunwind:

1. Resolve current `g_current_frame_slots` + count → mark each ptr-kind slot as a root.
2. `unw_step` to walk up the call stack; each frame's prologue saved its own slot table on the C stack.
3. Repeat until `unw_step` returns 0 (top of stack).

### Fallback: setjmp

If libunwind is unavailable at build time, fall back to:

1. Caller-side `setjmp(g_jmpbuf)` before each user-fn call → saves machine registers + caller's slot table.
2. Mark phase scans the saved jmp_buf for ptr-shaped i64 values; each is a candidate root.

This is conservative — every i64 that happens to look like an in-heap ptr gets marked. False positives leak memory but don't corrupt.

### Slot kind annotation

Each slot's `ValueKind` is statically known at codegen time. Only ptr-kind slots (String / Function / Table / TaggedValue with reference-tag payloads) need to be marked. Number / Bool / Nil slots are skipped.

The slot table layout:

```
struct SlotTable {
    i64 count;
    {ptr slot_addr, i8 kind_tag}[count];
}
```

### When stack walk is unsafe

Mark phase is **only safe** when no MLIR-emitted code is executing other than the `collectgarbage()` builtin itself. This is naturally the case from user Lua code (the call to `collectgarbage` is synchronous and returns before user code resumes). Coroutines (future ADR) will need a more sophisticated suspension mechanism.

## Alternatives considered

- **Conservative scan of the whole C stack** (no per-frame registry). Rejected — too many false positives; common register-spilled values (ints, flags) get mistaken for heap ptrs.
- **Shadow stack** (mirror every alloca write into a parallel data structure). Rejected — write barrier on every store is expensive.
- **Type-precise stack maps** (LLVM's `gc.statepoint` / `gc.relocate`). Rejected for v1 — requires LLVM-side GC support that's experimental for our pipeline.

## Consequences

**Positive**
- Per-frame registry gives precise root identification.
- libunwind fallback to setjmp keeps the design portable.
- Coroutine work has a clear extension point.

**Negative**
- Every user fn entry pays a slot-table write. Negligible in fn-call-heavy code, measurable in hot inner loops.
- Slot-table emission is a codegen change that touches every `emit_function`.
- Conservative setjmp fallback can leak.

## Future work

- Implementation commit.
- libunwind vs setjmp decision at build time (configure feature flag).
- Coroutine-aware suspension.
- LLVM `gc.statepoint` migration if profiling demands precise stack maps.

## References

- [ADR 0156](0156-gc-architecture-v1.md) — v1 root set deferred this.
- [ADR 0159](0159-gc-mark-phase.md) — mark phase that consumes these roots.
- libunwind project docs.
