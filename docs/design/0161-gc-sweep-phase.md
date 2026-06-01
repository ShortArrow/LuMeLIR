# 0161. Phase 3 GC step 5 — Sweep Phase Design

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Step 5 of ADR 0156's roadmap. Sweep is the final phase that turns the WHITE/BLACK mark from ADR 0159 into actual `free` calls. Decision-only.

## Decision

### Algorithm

Walk `g_gc_head` linked list with a prev cursor. For each node:

```
prev = null
cur = load(g_gc_head)
while cur != null:
    next = cur.next_field
    if cur.mark == BLACK:
        cur.mark = WHITE   // reset for next cycle
        prev = cur
    else: // WHITE
        if prev == null:
            store(next, g_gc_head)  // unlink from head
        else:
            store(next, prev.next_field)
        g_gc_total_bytes -= cur.size + GC_HEADER_SIZE
        free(cur)  // raw ptr — actual malloc returned this
    cur = next
```

The free call uses the **raw** ptr (with header), not the user-visible payload ptr. This is the original `malloc` return so `free` is correct.

### Safety

Sweep MUST run after mark. The chokepoint `collectgarbage()` codegen arm is:

```
emit_gc_mark()   // ADR 0159
emit_gc_sweep()  // this ADR
```

Until ADR 0160 wires real stack walk, ADR 0159's v1 safety mode pre-marks everything BLACK; sweep then resets BLACK → WHITE for the next cycle and frees nothing.

### `collectgarbage()` return

The Lua spec says `collectgarbage()` returns the bytes collected. Our impl tracks `g_gc_total_bytes` delta:

```
before = load(g_gc_total_bytes)
emit_gc_mark()
emit_gc_sweep()
after = load(g_gc_total_bytes)
return (before - after).f64
```

The 0-arg arm in ADR 0157 codegen (which currently returns 0.0) is replaced by this real flow when sweep lands.

### Finaliser interaction

When ADR 0163 lands (`__gc` finaliser), the sweep phase calls the finaliser BEFORE freeing the object. This ADR's algorithm reserves the hook point but the finaliser dispatch is implemented in 0163.

## Alternatives considered

- **Delete-and-rebuild** the list (not in-place unlink). Rejected — quadratic at scale.
- **Skip per-object size accounting**. Rejected — `collectgarbage("count")` becomes inaccurate.
- **Defer free to a worker thread**. Rejected for v1 — adds thread overhead; revisit if pause time matters.

## Consequences

**Positive**
- Linear pass, in-place unlink.
- `g_gc_total_bytes` stays accurate.
- Finaliser hook reserved cleanly.

**Negative**
- Stop-the-world pause proportional to heap size.

## Future work

- Implementation commit.
- Per-size-class free lists for reuse (optimisation).
- Concurrent sweep (only if pause-time profiling demands).

## References

- [ADR 0156](0156-gc-architecture-v1.md) — type tags + header.
- [ADR 0159](0159-gc-mark-phase.md) — mark phase paired with this.
- [ADR 0163](0163-gc-finaliser.md) — `__gc` finaliser hook.
