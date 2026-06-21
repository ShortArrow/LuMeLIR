# 0256. GC Capturing Closures as Roots (N2-C)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third N2 sub-ADR. [ADRs 0254 / 0255](0254-gc-table-array-propagation.md) gave Tables full GC tracking; the `chunk_safe_for_real_gc` predicate previously also rejected chunks with any **capturing** user function, sending those programs to v1 safety mode. The blocker was that capturing closures allocate heap closure cells + upvalue boxes (both `gc_alloc`-routed → in `g_gc_head`) but had no chunk-level root to keep them BLACK across `collectgarbage()`.

This ADR lifts that restriction:

1. Drop the `all_fns_non_capturing` predicate check.
2. Register `ValueKind::Function(_)` chunk slots as roots (in the existing `g_chunk_root_table`). Non-capturing slots stay null (their static `@<fn>_closure` global is reached directly without slot reads); capturing slots store the heap cell ptr — the root scan now keeps that cell BLACK.
3. Add a sibling propagation pass `emit_gc_mark_closure_cell_propagation_pass` that walks `g_gc_head`; for each obj that is BLACK AND `GC_TYPE_CLOSURE_CELL`, reads the cell's `upvalue_count` and marks each upvalue box BLACK via the existing membership-checked helper. Runs in the same 8-iteration fixpoint loop as the Table propagation (ADR 0255).

## Scope (literal)

- ✅ `chunk_safe_for_real_gc` no longer rejects capturing closures.
- ✅ Function-kind chunk slots register into `g_chunk_root_table` alongside String / Table slots. Null-init at `main` entry guarantees the root scan stays a safe no-op for non-capturing slots.
- ✅ New propagation pass marks BLACK closure cells' upvalue boxes BLACK. Boxes themselves are `GC_TYPE_UPVALUE_BOX`-tagged allocations in `g_gc_head`; the membership check finds them.
- ✅ Runs every fixpoint iteration alongside the Table propagation — closure→Table-upvalue chains marked transitively across iterations.
- ❌ **Following upvalue box CONTENTS.** Without per-upvalue type metadata at runtime, an upvalue box holding an i64 value is ambiguous: it could be a packed Number (no marking needed) OR a String/Table ptr (must mark). Marking arbitrary i64s as ptrs would mark non-GC memory. **Bounded leak**: a String / Table held only as a closure upvalue stays alive (won't crash) but isn't tracked transitively — if the user code releases all explicit refs to that String / Table, sweep won't free it until the closure itself dies. Future ADR can add per-upvalue-slot type metadata (4 bits in the box header) or convert boxes to 16-byte tagged slots.
- ❌ Mutual-recursive capturing closures' cycle handling. The fixpoint pass marks reachable cells but doesn't detect cycles; the membership check makes the cost O(N²) but correctness is preserved.
- ❌ Function-kind args inside user fn bodies (params). Per-frame stack walk for non-chunk Function locals is N2-D scope.

## Decision

### Root table extension

Function-kind chunk Locals join String + Table in the chunk root table population (`emit.rs` ~2700). The slot's content (a ptr — static closure global addr or heap cell user_ptr) is compared to `g_gc_head` user_ptrs by the root scan. Non-capturing fn slots stay null (LocalInit alias-skip path emit.rs:3418) → no match → no-op. Capturing fn slots hold the cell ptr → match → mark BLACK.

### Closure cell propagation

```mlir
for each obj in g_gc_head:
  if obj.mark == BLACK and obj.type_tag == GC_TYPE_CLOSURE_CELL:
    cell_user = obj + GC_HEADER_SIZE
    upv_count = load(cell_user + CLOSURE_OFF_UPVALUE_COUNT)
    for i in 0..upv_count:
      box_ptr = load(cell_user + CLOSURE_OFF_BOXES_BASE + i*8)
      mark_user_ptr_black_if_nonnull(box_ptr)
```

The mark helper's existing g_gc_head membership check guards against the `null` `box_ptr` case (initial alloca state) and confirms each box's tracked-allocation status before writing.

### Why fixpoint composition is correct

The 8-iteration outer loop (ADR 0255) runs both passes per iteration. After iteration 1: roots → BLACK Tables + BLACK closure cells, plus their direct (single-level) children. After iteration 2: those children's children also BLACK. By iteration 8 anything reachable up to depth 8 is BLACK. Closure-cell → upvalue-box → (future) box contents would chain through the same loop.

## Tests

`tests/phase4_n2c_capturing_closures_gc.rs` (NEW, 5 e2e):

1. Counter closure with Number upvalue survives collection mid-loop (1, 2, gc, 3, 4).
2. Returned closure (`make_adder(3)` ↦ `add3`) can be called after gc.
3. Single-param-upvalue closure works.
4. Two independent closures each keep their own state across gc.
5. Unrelated transient gets freed while closure stays live (positive `freed` delta).

## Test count delta

```
Step 0:  1630 (after ADR 0255)
N2-C (impl + 5 e2e): 1630 → 1635
```

## What this unblocks

- Programs using the classic Lua "factory function returning a closure" pattern now run with real GC freeing instead of v1 safety mode.
- M9-C (`__close` runtime hook) gains a path: scope-exit hook can rely on the closure cell + box GC infrastructure.
- N2-D (per-frame stack walk for non-chunk Function locals) can now compose with the closure-cell propagation when functions hold inner functions as locals.

Still gated:
- Upvalue-box CONTENT marking. Strings / Tables held only via upvalues stay alive but aren't transitively freed when their owning closure dies until N3 (after per-upvalue type metadata lands).

## References

- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell + upvalue box layout.
- [ADR 0254](0254-gc-table-array-propagation.md) — N2-A sibling.
- [ADR 0255](0255-gc-table-hash-and-nested.md) — N2-B sibling with the 8-iteration fixpoint loop reused here.
- [ADR 0218](0218-gc-chunk-safe-real-freeing.md) — chunk-safe predicate (this ADR removes its capturing-closure clause).
- [ADR 0157](0157-phase3-gc-allocator-wrapper.md) — `gc_alloc` chokepoint that closure cells route through.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N2-C in the N1-N10 path.
