# 0159. Phase 3 GC step 3 — Mark Phase Design

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Step 3 of ADR 0156's roadmap. With the `g_gc_head` linked list complete (ADR 0158), the mark phase can begin its DFS through the heap. This ADR pins the algorithm and per-type reference walks; implementation stages on top in a follow-up commit.

Decision-only — closes the design surface; implementation lands when the trigger fires.

## Decision

### Mark algorithm

Iterative tri-color DFS. WHITE=0 (unreached), GREY=1 (in worklist), BLACK=2 (fully traversed).

```
worklist = roots
for r in roots: r.mark = GREY
while worklist not empty:
    obj = pop(worklist)
    if obj.mark == BLACK: continue
    obj.mark = BLACK
    push all references from obj into worklist (mark each GREY)
```

### Root set (v1)

| Source | How reached |
|---|---|
| Module-level string globals (typename / diagnostic msgs / user literals) | Walk the static string pool registered at `emit_module_init`; each entry's payload ptr is a root. |
| Closure singleton globals (`@<fn>_closure`) | Each user function declared with `function_names` registers a `<sym>_closure` global; iterate at runtime via a per-module `mod_init_root_list` (registered at emit time). |
| Top-level alloca slots (`main`'s locals) | **Deferred to ADR 0160.** In v1 the mark phase treats the top-level chunk locals as "ambient live" — all allocations made before the first `collectgarbage()` call are pre-marked BLACK to match Phase 2 leak semantics. |
| Inner-frame alloca slots | **Deferred to ADR 0160.** v1 collection is safe only when no nested function frame is active (i.e. only at top-level `collectgarbage()` call sites). |

### Per-type reference walks

| Type tag | References to push |
|---|---|
| `GC_TYPE_TABLE` | `array_buf`, `hash_buf`, `metatable_ptr` (all from header offsets 16 / 24 / 32). |
| `GC_TYPE_ARRAY_BUF` | Each occupied slot (tag != TAG_NIL) at indices 0..length; if slot's tag is String / Function / Table, push slot.payload. |
| `GC_TYPE_HASH_BUF` | Each non-empty, non-DELETED entry (tag != TAG_NIL, != TAG_DELETED) at indices 0..cap; push both key.payload AND value.payload when reference kind. |
| `GC_TYPE_STRING_OBJ` | No outgoing references; mark BLACK. |
| `GC_TYPE_SCRATCH_BUF` | No outgoing references; mark BLACK. |
| `GC_TYPE_CLOSURE_CELL` | Each `upvalues[i]` slot ptr (closure layout, ADR 0083); push as upvalue box. |
| `GC_TYPE_UPVALUE_BOX` | If box's kind is String / Function / Table, push box.payload. |

### Algorithm chokepoint

New runtime helper `emit_gc_mark()` walks the worklist. Implementation lives in a single `func.func @gc_mark()` that:
1. Loads `g_gc_head` to find first allocation.
2. Iterates with a worklist (alloca-backed stack of i64 capacity 4096; deeper graphs panic — bumped in follow-up if needed).
3. Per-type switch via tag at GC_HEADER_OFF_TYPE_TAG.

`collectgarbage()` codegen arm calls `gc_mark` followed by `gc_sweep` (ADR 0161).

### v1 safety mode

Until ADR 0160 wires stack walk, `collectgarbage()` will **first** scan `g_gc_head` and **mark every allocation BLACK** before the DFS starts. The DFS then verifies the graph is consistent but no objects become WHITE. Sweep is a no-op in this safety mode.

Once ADR 0160 lands and real roots are available, the safety mark is removed; only actual roots become GREY/BLACK.

## Alternatives considered

- **Skip mark phase entirely until 0160.** Rejected — having the algorithm in place lets the structural code review happen now, and the safety mode keeps Phase 3 v1 binaries behaving like Phase 2 (leak).
- **Recursive DFS instead of iterative worklist.** Rejected — table chains can exceed stack depth.
- **Per-type linked lists** so each type can be walked independently. Rejected — single list keeps the architecture simple; 0156 locked this in.

## Consequences

**Positive**
- Mark phase design pinned for implementation.
- Per-type reference graph documented in one place.
- Safety mode allows Phase 3 v1 to ship without ADR 0160.

**Negative**
- v1 safety mode means `collectgarbage()` is observably a no-op until ADR 0160 lands.
- The 4096-deep worklist is a soft cap that doesn't grow.

## Pre-implementation note (2026-06-13)

Pre-flight review (Codex 6 視点) flagged two points to resolve before implementation:

- **R1 — worklist capacity strategy.** The 4096-cap alloca worklist mentioned for v1 is an ad-hoc placeholder. Recommended fix in the implementation ADR: heap-based `malloc` worklist that grows on demand, allocated outside `emit_gc_alloc` so it is not GC-tracked. Eliminates the silent saturation risk on deep `__index` chains (ADR 0167 multi-hop).
- **R2 — per-type dispatch chokepoint.** The per-type reference walk (§Per-type reference walks) is the first user of a `GC_TYPE_*` switch. Before mark / sweep / future debug paths each open-code the same `match`, extract a `gc_type_references(type_tag) -> &'static [GcReferenceField]` decision table in `tagged.rs`. Tidy First — precedent in ADR 0182 (`mark_ident_as` consolidation done with only two consumers).

Both items land as **preparatory ADR 0184** (`gc_type_references` decision table) before the mark phase implementation in **ADR 0185**.

Full review and rationale: [`docs/notes/gc-0159-0162-preflight-review.md`](../notes/gc-0159-0162-preflight-review.md).

## Future work

- Implementation commit per this design.
- Worklist grow strategy (folded into ADR 0184/0185 per pre-impl note above).
- Incremental marking (only if profiling demands).

## References

- [ADR 0156](0156-gc-architecture-v1.md) — type tags, header layout.
- [ADR 0157](0157-gc-allocator-wrapper.md) / [ADR 0158](0158-gc-migrate-remaining-types.md) — allocator migrations.
- [ADR 0160](0160-gc-stack-walk.md) — stack walk that v1 safety mode masks.
- [ADR 0161](0161-gc-sweep-phase.md) — sweep paired with this mark.
