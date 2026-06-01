# 0163. Phase 3 GC step 7 — `__gc` Finaliser

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Step 7 of ADR 0156's roadmap. Lua spec §2.5.3: when a table with a `__gc` metamethod becomes unreachable, the GC calls `mt.__gc(t)` before freeing. Decision-only.

## Decision

### Hook point

In ADR 0161's sweep loop, for each WHITE object:

```
if cur.type_tag == GC_TYPE_TABLE:
    mt_ptr = load(cur + TABLE_OFF_METATABLE)
    if mt_ptr != null:
        probe mt_ptr for "__gc"
        if probe.tag == TAG_FUNCTION:
            call probe with [cur as Table arg]
free(cur)
```

### Signature

`__gc(t)` per spec: `(Table) → ()`. No return. Compile-time candidate filter selects user fns with that signature; if none, `__gc` is silently no-op (matches Lua spec — a non-Function `__gc` is also no-op).

### Resurrection

Lua spec allows `__gc` to "resurrect" an object by storing it into a reachable location. v1 deferral: the resurrected object stays in `g_gc_head` but is marked WHITE next cycle and re-collected (or re-marked BLACK if a new root reaches it). Multiple `__gc` calls on the same object are NOT prevented in v1; document this deviation.

### Resurrection-during-finaliser safety

Free is deferred until ALL finalisers in the current sweep cycle have run. This avoids the case where `__gc(t1)` resurrects t2, t2 then gets finalised in the same cycle, t1's finaliser thinks t2 is dead. Algorithm:

1. First pass: walk list, collect `(obj_ptr, finaliser_fn)` pairs for WHITE objects with `__gc`.
2. Run all finalisers.
3. Second pass: walk list again, free WHITE objects.

### Registration

A table only needs finalisation if its metatable has `__gc` at the time of marking. v1 checks at sweep time (not registration time). Future v2 may add a "needs finalisation" header bit set at `setmetatable` time for efficiency.

## Alternatives considered

- **Skip resurrection support entirely.** Rejected — Lua spec explicitly allows it; absence breaks compatibility.
- **Per-finaliser-cycle resurrection prevention** (mark finalised objects as "done" and never re-finalise). Lua-spec-compliant but deferred to v2.
- **Eagerly run finalisers at WHITE-detection time** rather than two-pass. Rejected — resurrection race described above.

## Consequences

**Positive**
- Standard `__gc` idiom (e.g. close file handles, release locks) works.
- Two-pass sweep is straightforward.

**Negative**
- Two-pass sweep doubles the walk.
- Resurrection corner cases not all covered in v1.

## Future work

- Implementation commit.
- "Needs finalisation" header bit (perf optimisation).
- Resurrection-prevention bit per Lua 5.4 §2.5.3.

## References

- [ADR 0156](0156-gc-architecture-v1.md) — GC header reserves padding for future bits.
- [ADR 0142](0142-tostring-metamethod.md) — metamethod dispatch helper reused.
- [ADR 0161](0161-gc-sweep-phase.md) — sweep loop this hooks into.
- Lua 5.4 reference manual §2.5.3 — finalisers.
