# 0185. GC Mark + Sweep — v1 Safety Mode Implementation

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-13
- **Deciders:** ShortArrow

## Context

Phase 3 GC ADRs 0156 / 0157 / 0158 / 0184 landed the allocator wrapper (`emit_gc_alloc`), migrated every malloc site through it, and added the per-type metadata chokepoint plus size guard. ADRs 0159 (mark) / 0161 (sweep) / 0162 (auto-trigger) are decision-only.

ADR 0159's "v1 safety mode" (§v1 safety mode, line 62) is the principled bridge that lets mark + sweep land before ADR 0160's stack walk: every `collectgarbage()` call first marks every `g_gc_head` entry BLACK, the DFS becomes a no-op, sweep finds nothing WHITE, and the heap is unchanged. The full DFS / per-type walks / worklist (R1 from the pre-flight memo) live with ADR 0187 once real roots are available.

This ADR implements that v1 safety mode. Observable user-visible behaviour is unchanged (`collectgarbage()` still returns 0; `g_gc_total_bytes` is unchanged across collection); what gains is structural completeness — the `emit_gc_mark` / `emit_gc_sweep` chokepoints exist, the `collectgarbage()` Lua builtin wires through them, and the next ADR plugs real roots into the same pipeline.

## Scope (literal)

- ✅ `emit_gc_mark()` MLIR function: single linear walk of `g_gc_head` setting `*(obj + GC_HEADER_OFF_MARK) = BLACK` for every node. No worklist, no per-type recursion in v1.
- ✅ `emit_gc_sweep()` MLIR function: walk `g_gc_head` with prev cursor (per ADR 0161 algorithm), free WHITE entries (none exist in v1 safety mode but the code path is exercised), reset BLACK → WHITE, maintain `g_gc_total_bytes -= size + GC_HEADER_SIZE` accounting on each free.
- ✅ Rewire `Callee::Builtin(CollectGarbage)` emit arm: when `args.len() == 0`, replace the existing `0.0` constant stub with the load-before / mark / sweep / load-after / subtract / sitofp sequence per ADR 0161 §`collectgarbage()` return.
- ✅ `BLACK` / `WHITE` mark byte constants added to `tagged.rs` next to `GC_HEADER_OFF_MARK`.
- ✅ 3 Red Day 0 e2e (regression pins, since v1 safety mode is observably no-op): `collectgarbage()` returns 0; `count` is unchanged across collection; sequential `collectgarbage()` calls do not corrupt the heap.
- ❌ Worklist + per-type DFS (R1). Deferred to ADR 0187 alongside stack walk.
- ❌ Real root marking. Deferred to ADR 0187.
- ❌ Auto-trigger threshold (ADR 0162 implementation). Deferred to ADR 0186 — independent, can land before or after 0187.
- ❌ `collectgarbage("stop")` / `"restart"` / `"step"` / `"setpause"` / `"setstepmul"`. Still rejected at HIR per ADR 0157.
- ❌ Finalizer dispatch / `__gc`. Deferred to ADR 0163.

## Decision

### `src/codegen/tagged.rs`

Add mark colour constants alongside `GC_HEADER_OFF_MARK`:

```rust
/// ADR 0185 — v1 mark colour values stored in
/// `*(obj + GC_HEADER_OFF_MARK)`. Mark phase sets BLACK; sweep
/// resets BLACK → WHITE for the next cycle. GREY is reserved
/// for ADR 0187 worklist DFS.
pub(crate) const GC_MARK_WHITE: i64 = 0;
#[allow(dead_code)] // first consumer: ADR 0187 (worklist DFS)
pub(crate) const GC_MARK_GREY: i64  = 1;
pub(crate) const GC_MARK_BLACK: i64 = 2;
```

The existing `emit_gc_alloc` zero-byte mark init (line ~405 in `primitive.rs`) already corresponds to `GC_MARK_WHITE`. No change to allocation.

### `src/codegen/emit.rs` — new helpers

#### `emit_gc_mark_func`

Emits a `func.func @gc_mark()` once per module (registered at module init alongside the other Phase 3 globals). Body:

```
cur_iv = load(g_gc_head, i64)
scf.while (carrier: i64 = cur_iv) {
    cond: cur_iv != 0
    body:
        cur_ptr = inttoptr(cur_iv, ptr)
        // v1 safety mode: mark every obj BLACK.
        mark_ptr = byte_offset(cur_ptr, GC_HEADER_OFF_MARK)
        store(BLACK_i8, mark_ptr)
        next_ptr = byte_offset(cur_ptr, GC_HEADER_OFF_NEXT)
        next_val = load(next_ptr, ptr)
        next_iv = ptrtoint(next_val, i64)
        yield next_iv
}
return
```

#### `emit_gc_sweep_func`

Emits a `func.func @gc_sweep()` once per module. Body follows ADR 0161 algorithm verbatim:

```
prev_iv = 0_i64
cur_iv  = load(g_gc_head, i64)
scf.while (carrier: (prev_iv, cur_iv)) {
    cond: cur_iv != 0
    body:
        cur_ptr   = inttoptr(cur_iv, ptr)
        next_ptr  = byte_offset(cur_ptr, GC_HEADER_OFF_NEXT)
        next_val  = load(next_ptr, ptr)
        next_iv   = ptrtoint(next_val, i64)

        mark_ptr  = byte_offset(cur_ptr, GC_HEADER_OFF_MARK)
        mark_byte = load(mark_ptr, i8)
        is_black  = cmpi(eq, mark_byte, BLACK_i8)

        scf.if is_black:
            // Reset BLACK → WHITE; keep node in list.
            store(WHITE_i8, mark_ptr)
            yield (cur_iv, next_iv)     // prev = cur
        else:
            // WHITE → unlink, decrement total_bytes, free raw ptr.
            size_ptr = byte_offset(cur_ptr, GC_HEADER_OFF_SIZE)
            size_u32 = load(size_ptr, i32)
            size_i64 = zextu(size_u32, i64)
            total_minus = size_i64 + GC_HEADER_SIZE
            bytes_addr  = addressof(g_gc_total_bytes)
            old_bytes   = load(bytes_addr, i64)
            new_bytes   = old_bytes - total_minus
            store(new_bytes, bytes_addr)

            scf.if prev_iv == 0:
                // Unlink from head.
                store(next_val, addressof(g_gc_head))
            else:
                prev_ptr      = inttoptr(prev_iv, ptr)
                prev_next_ptr = byte_offset(prev_ptr, GC_HEADER_OFF_NEXT)
                store(next_val, prev_next_ptr)

            libc::free(cur_ptr)
            yield (prev_iv, next_iv)    // prev unchanged
}
return
```

Both helpers go in `src/codegen/emit.rs` near the other Phase 3 GC globals, behind a small `register_gc_runtime_funcs(context, module, ...)` entry point that `emit_module_init` calls once.

#### `Callee::Builtin(CollectGarbage)` emit arm

The current `args.len() == 0` arm (per ADR 0157) just emits `arith.constant 0.0 : f64`. Replace with:

```rust
let bytes_addr = emit_addressof(context, block, "g_gc_total_bytes", types, loc);
let before     = emit_load(block, bytes_addr, types.i64, loc);
emit_libc_call_void(context, block, "gc_mark",  &[], loc);
emit_libc_call_void(context, block, "gc_sweep", &[], loc);
let after      = emit_load(block, bytes_addr, types.i64, loc);
let delta      = emit_isub(block, before, after, loc);
let result_f64 = emit_i2f(block, delta, loc);
```

`emit_libc_call_void` exists for void-returning external calls (e.g. `exit`); the `gc_mark` / `gc_sweep` functions are local module functions, not libc — actual call uses `func::call` builder. The pseudo-code above is approximate; implementation step picks the precise builder.

The `args.len() == 1` `count` arm is unchanged.

### Tests

`tests/phase3_gc_mark_sweep_v1.rs` (NEW, 3 e2e):

1. `collectgarbage()` returns `0.0` after a no-op program (v1 safety mode delta = 0).
2. `collectgarbage("count")` is unchanged before and after a `collectgarbage()` call (v1 safety mode frees nothing).
3. Two back-to-back `collectgarbage()` calls do not corrupt heap (subsequent allocations succeed, `count` continues to accumulate).

All three are regression pins. They are Red against the current `args.len() == 0` stub if and only if the new mark / sweep code introduces a crash or accounting bug; otherwise they observe identical behaviour. The TDD signal here is "absence of subtle breakage", not feature presence.

## Alternatives considered

- **Skip the safety-mode walk and leave `collectgarbage()` as a stub until 0187.** Rejected — the safety-mode pass exercises every line of the mark + sweep walk, catching bugs in the linked-list cursor / accounting before real roots arrive. Without it, ADR 0187 would land "first DFS + first sweep" together; the surface is too large for one review.
- **Mark `WHITE` rather than `BLACK` in safety mode** so sweep is exercised for the "free a WHITE entry" branch. Rejected — would actually free objects that may be referenced from top-level alloca slots (no stack walk yet) → use-after-free at runtime. The whole point of safety mode is to be conservative.
- **Inline mark and sweep at every `collectgarbage()` call site.** Rejected — the call site is shared (`collectgarbage()` Lua builtin); inlining bloats code with no payoff. `func.func` boundaries here also help mark / sweep show up legibly in `--emit-llvm` debugging.
- **Implement worklist + per-type DFS now even though it's unused in safety mode.** Rejected — the worklist capacity strategy (R1) is its own design choice; bundling it here without exercise (safety mode pre-marks BLACK so the DFS visits nothing) buys nothing.

## Consequences

**Positive**
- `collectgarbage()` is no longer a stub — the full mark + sweep pipeline runs.
- `gc_mark` / `gc_sweep` chokepoints exist for ADR 0187 to extend (add real DFS body / consume `gc_type_meta`).
- Heap accounting (`g_gc_total_bytes`) reaches every code path it will need.
- v1 safety mode honors the Phase 2 leak contract (ADR 0145) — nothing observable changes for user programs.

**Negative**
- One extra linear walk of `g_gc_head` per `collectgarbage()` call (the BLACK pre-mark). O(n) per collection; v1 collections are user-initiated so the cost is opt-in.
- `gc_mark` / `gc_sweep` are MLIR-level functions; first-time module init has two more function emissions. ~negligible.

**Locked in until superseded**
- Mark colour constants (`GC_MARK_WHITE = 0`, `GC_MARK_GREY = 1`, `GC_MARK_BLACK = 2`) live in `tagged.rs`. ADR 0187 reuses them.
- `gc_mark` / `gc_sweep` function names / signatures are the public stable contract for ADR 0187 to extend in-place.

## Documentation updates

- [x] §8 — adds 0185.
- [x] ADR 0159 pre-impl note — v1 safety mode implementation DONE here; worklist + DFS remain for ADR 0187.
- [x] ADR 0161 pre-impl note — sweep algorithm implementation DONE here.
- [x] ADR 0162 status reminder — auto-trigger threshold still unimplemented; tracked for ADR 0186.

## Test count delta

```
Step 0: 1412 (after ba1e5f3)
C1 (doc): 1412 → 1412
C2 (3 Red Day 0 regression pins): 1412 → 1415 (all green from day 0 if no bug; Red if the new code crashes/miscounts)
C3 (impl): 1415 → 1415
```

## Critical files

- `src/codegen/tagged.rs`:
  - Add `GC_MARK_WHITE` / `GC_MARK_GREY` (allow dead_code) / `GC_MARK_BLACK` constants.
- `src/codegen/emit.rs`:
  - Add `emit_gc_mark_func` + `emit_gc_sweep_func` (registered from `emit_module_init`).
  - Rewire `Callee::Builtin(CollectGarbage)` `args.len() == 0` arm to mark / sweep + return delta.
- `tests/phase3_gc_mark_sweep_v1.rs` (NEW) — 3 e2e regression pins.

## Risks

| Risk | Mitigation |
|---|---|
| Sweep frees a node still referenced from an alloca slot | v1 safety mode pre-marks every node BLACK before sweep; no WHITE node exists; the "free" branch is exercised but dead-in-practice. ADR 0187 lifts safety mode only after stack walk delivers real roots. |
| `g_gc_total_bytes` underflow on the free path | Free branch only runs for WHITE nodes; in v1 safety mode no node is WHITE so the path is unreachable in practice. When ADR 0187 lifts the safety mode, a saturated subtract or assert keeps overflow off the table. |
| `func.func @gc_mark` redefinition on multi-compile in same MLIR module | `emit_module_init` is called once; the helpers are emitted once per module. Same lifecycle as existing `emit_string_runtime_decls`. |
| Test 2 (count unchanged) fails if the safety mode is implemented incorrectly | Test is exactly the v1 contract pin — failure means safety mode bug, not test wrongness. |

## Future work

- ADR 0187 — Worklist + per-type DFS (consumes `gc_type_meta(tag).has_outgoing_refs` short-circuit; lifts safety mode once stack walk lands). Resolves R1.
- ADR 0186 — Auto-trigger threshold per ADR 0162 (independent; can land before or after 0187).
- ADR 0188 — Stack walk per ADR 0160; pairs with 0187 to enable real freeing.

## References

- [ADR 0156](0156-gc-architecture-v1.md) — parent architecture.
- [ADR 0157](0157-gc-allocator-wrapper.md) — `emit_gc_alloc`; mark byte initialised to WHITE there.
- [ADR 0159](0159-gc-mark-phase.md) — mark phase design (this ADR implements the v1 safety-mode subset).
- [ADR 0161](0161-gc-sweep-phase.md) — sweep algorithm (this ADR implements it verbatim).
- [ADR 0184](0184-gc-type-meta-and-size-guard.md) — preparatory metadata chokepoint.
- [`docs/notes/gc-0159-0162-preflight-review.md`](../notes/gc-0159-0162-preflight-review.md) — pre-flight review.
