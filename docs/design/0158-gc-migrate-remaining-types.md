# 0158. Phase 3 GC step 2 — Migrate Remaining Allocator Types

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

[ADR 0157](0157-gc-allocator-wrapper.md) migrated the first allocator type (string objects). This ADR migrates the remaining 6 GC-managed allocator types so the `g_gc_head` linked list captures every heap object. After this ADR, the list is complete and the next ADR (0159 mark phase) has a full reference graph to walk.

## Scope (literal)

Migrate every site that calls `emit_libc_call_ptr("malloc", &[size])` for a GC-managed allocation. 8 sites total:

| Site | Allocator type | type_tag |
|---|---|---|
| `emit_table_array_grow` (`emit.rs:5866`) | Array buf | `GC_TYPE_ARRAY_BUF` |
| `emit_hash_ensure_buf` (`emit.rs:6017`) | Hash buf (initial) | `GC_TYPE_HASH_BUF` |
| `emit_hash_grow_if_needed` (`emit.rs:6951`) | Hash buf (regrow) | `GC_TYPE_HASH_BUF` |
| Table constructor header alloc (`emit.rs:7481`) | Table | `GC_TYPE_TABLE` |
| Table constructor array buf (`emit.rs:7514`) | Array buf | `GC_TYPE_ARRAY_BUF` |
| Snprintf scratch (`emit.rs:9216`) | Scratch buf | `GC_TYPE_SCRATCH_BUF` |
| `emit_tostring(Number)` scratch (`emit.rs:14045`) | Scratch buf | `GC_TYPE_SCRATCH_BUF` |
| Closure cell alloc (`closure.rs:236`, `337`) | Closure cell / Upvalue box | `GC_TYPE_CLOSURE_CELL` / `GC_TYPE_UPVALUE_BOX` |

Out of scope:

- ❌ Mark phase (ADR 0159).
- ❌ Sweep phase (ADR 0161).
- ❌ Snprintf scratch buffer migration *invariant* check — ADR 0145 noted Phase 3's GC trigger threshold should not include short-lived scratch buffers; pragmatically we still track them (sweep will reclaim immediately on next collection).
- ❌ Other allocator categories that aren't in the list above.

## Decision

Each call site changes from:

```rust
emit_libc_call_ptr(context, block, "malloc", &[size], types, loc)
```

to:

```rust
emit_gc_alloc(context, block, size, GC_TYPE_<X>, types, loc)
```

The user-visible returned ptr stays at the same offset relative to the payload (i.e. `raw + 16`), so existing offset constants (`TABLE_OFF_LEN = 0`, etc.) stay correct.

For closure cells (ADR 0083): `emit_allocate_closure_cell` and `emit_allocate_upvalue_box` are the two helpers; both gain the new tag.

## Alternatives considered

- **Migrate site-by-site across multiple ADRs.** Rejected — each site is a 1-line change, batching keeps the linked-list-incomplete window short.
- **Skip scratch buffers** (the snprintf scratches are short-lived). Rejected — partial coverage breaks the "every malloc is tracked" invariant the mark/sweep phases rely on. Mark phase will see scratch ptrs unreachable from roots and sweep will reclaim them on next collection — exact-correct semantics.
- **Add a new helper per allocator type** (`emit_table_alloc` / `emit_hash_buf_alloc` / etc.). Rejected — `emit_gc_alloc` is the single chokepoint; adding wrappers spreads the type-tag knowledge.

## Consequences

**Positive**
- `g_gc_head` linked list now contains every heap-allocated GC-managed object.
- `collectgarbage("count")` reports the true heap size.
- ADR 0159 (mark phase) can begin with full graph coverage.

**Negative**
- 8 sites touched. Each is a 1-line change but verification surface scales.
- 16 bytes of header overhead per allocation now applies to all types (tables, hash buffers, etc.). For tiny tables `{}` this is significant (40 + 16 = 56 bytes total).

**Locked in until superseded**
- All 8 sites use the same `emit_gc_alloc` chokepoint.
- `closure.rs` migrations don't change the closure cell layout.

## Documentation updates

- [x] §4 LIC — new `LIC-gc-migrate-remaining-types-1`.
- [x] §8 — adds 0158.

## Test count delta

```
Step 0:   1369 (after ADR 0157)
C2 (1 e2e + regression):  1369 → 1369
C3 (impl): 1369 → 1370
```

## Critical files

- `src/codegen/emit.rs`:
  - 7 call sites migrated to `emit_gc_alloc(... type_tag ...)`.
- `src/codegen/closure.rs`:
  - 2 call sites migrated.
- `tests/phase3_gc_migrate.rs` (NEW) — 1 e2e (table alloc bumps `count`).
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| One of the 8 site signatures differs and breaks build | Rust type-check catches every signature mismatch. |
| Header-offset overhead breaks existing offset constants | Header lives BEFORE the user-visible ptr; payload offsets stay correct (mirrors ADR 0157's string-object verification). |
| `emit_table_grow_if_needed` realloc path needs special handling | The current code mallocs a new buffer + memcpy + frees the old — same as ADR 0157's allocations, just one more site. |
| Tests pin behaviour at the ABI level, not the allocator level | Tests pass by observing `collectgarbage("count")` increase; regression tests prove no functional change. |

## Future work

- ADR 0159 — mark phase.
- ADR 0160 — stack walk.
- ADR 0161 — sweep phase.
- Profile heap overhead and decide if size-class buckets are worth adding later.

## References

- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell + upvalue box allocator sites.
- [ADR 0156](0156-gc-architecture-v1.md) — type-tag spec.
- [ADR 0157](0157-gc-allocator-wrapper.md) — `emit_gc_alloc` helper used here.
