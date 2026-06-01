# 0162. Phase 3 GC step 6 — Automatic Trigger Threshold

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Step 6 of ADR 0156's roadmap. ADR 0145 specified "1 MB or explicit `collectgarbage()`" as the trigger; this ADR pins the mechanism. Decision-only.

## Decision

### Threshold

Static threshold: **1,048,576 bytes (1 MiB)** of `g_gc_total_bytes`. When `emit_gc_alloc` would push the counter past this value, it inlines a call to `gc_mark` + `gc_sweep` first.

### Inline check

`emit_gc_alloc` extended at the end of its body:

```
new_total = g_gc_total_bytes + total_size
if new_total >= GC_TRIGGER_THRESHOLD:
    gc_mark()
    gc_sweep()
g_gc_total_bytes = new_total  // updated after sweep so post-collection count is right
```

The check is a single load + cmpi + scf.if; cheap on the fast path (the if's then-branch is just a yield).

### After-sweep growth

If sweep doesn't reclaim enough to drop below the threshold (large live working set), the trigger fires again on the next alloc → tight loop of collections. To avoid this, after sweep the threshold doubles for the current process (capped at 1 GiB):

```
if (post_sweep_total * 2) > GC_TRIGGER_THRESHOLD:
    GC_TRIGGER_THRESHOLD = min(post_sweep_total * 2, 1 GiB)
```

Threshold is itself stored in a mutable module global `g_gc_threshold` (init 1 MiB).

### Configurability

Future ADR may expose `collectgarbage("setpause", n)` to configure the doubling multiplier. For v1 the doubling is hardcoded.

## Alternatives considered

- **Per-allocation cycle count instead of bytes.** Rejected — small allocs would trigger too often, large allocs too rarely.
- **Allocator-rate trigger** (collect every N allocs). Rejected — same uniformity problem.
- **No automatic trigger; explicit only.** Rejected — defeats the point of having a GC.
- **Generational trigger** (collect nursery often, old gen rarely). Deferred per ADR 0145.

## Consequences

**Positive**
- Predictable trigger behaviour.
- Adaptive doubling prevents thrashing.

**Negative**
- 1 MiB is a guess; real workloads may want larger.
- Each `emit_gc_alloc` pays a small constant-time check.

## Future work

- Implementation commit.
- `collectgarbage("setpause", n)` builtin.
- Generational nursery (only if profiling demands).

## References

- [ADR 0145](0145-gc-strategy.md) — pinned the 1 MiB threshold.
- [ADR 0157](0157-gc-allocator-wrapper.md) — `emit_gc_alloc` extended here.
- [ADR 0159](0159-gc-mark-phase.md) / [ADR 0161](0161-gc-sweep-phase.md) — the cycle the trigger runs.
