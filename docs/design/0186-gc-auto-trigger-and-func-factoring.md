# 0186. GC Auto-Trigger Threshold + `llvm.func` Factoring of Mark/Sweep

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-13
- **Deciders:** ShortArrow

## Context

[ADR 0185](0185-gc-mark-sweep-v1-safety-mode.md) inlined the v1 safety-mode mark + sweep walks into the `Callee::Builtin(CollectGarbage)` emit arm with the explicit deferral: "Factoring into top-level `func.func @gc_mark` / `@gc_sweep` is deferred to ADR 0186 (auto-trigger) when a second caller emerges — same Tidy First 'extract before the second consumer' rule ADR 0182 applied to `mark_ident_as`."

That second caller is ADR 0162 — the automatic 1 MiB threshold trigger inside `emit_gc_alloc`. Every allocation site (8 per ADR 0158) would call the walks if they fire; inlining a ~400-LOC walk at each site is unacceptable bloat. So this ADR does both at once: factor the walks into `llvm.func @gc_mark` / `@gc_sweep` and add the threshold trigger that consumes them.

ADR 0162 was decision-only; this ADR implements it.

## Scope (literal)

- ✅ Factor `emit_gc_mark_inline` / `emit_gc_sweep_inline` (introduced by ADR 0185) into module-level `llvm.func @gc_mark()` and `llvm.func @gc_sweep()`. Same walk algorithms, same v1 safety-mode semantics — purely a Tidy First refactor.
- ✅ Rewire `Callee::Builtin(CollectGarbage)` 0-arg arm: replace the two inline-helper calls with `llvm.call @gc_mark()` / `llvm.call @gc_sweep()`.
- ✅ New mutable i64 global `g_gc_threshold`, init = `GC_THRESHOLD_INIT = 1,048,576` (1 MiB) — both constants live in `tagged.rs`.
- ✅ `emit_gc_alloc` extended at entry (before the existing OOM check / header init / total_bytes update) with:
  - Load current `g_gc_total_bytes`, compute hypothetical `new_total = current + total_size`.
  - Compare `new_total >= g_gc_threshold`.
  - If yes: `llvm.call @gc_mark()`, `llvm.call @gc_sweep()`, run the doubling logic on the post-sweep total.
  - Then proceed with the existing allocation.
- ✅ Doubling logic: after sweep, if `post_sweep_total * 2 > g_gc_threshold`, update `g_gc_threshold = min(post_sweep_total * 2, GC_THRESHOLD_CAP = 1 GiB)`.
- ✅ Module init registers both `g_gc_threshold` and the two `llvm.func`s alongside the existing GC globals.
- ✅ 3 regression-pin e2e: post-threshold allocation loop does not crash; `collectgarbage("count")` is consistent after auto-triggers fire; explicit `collectgarbage()` still returns 0 in v1 safety mode (auto-trigger does not alter the contract).
- ❌ `collectgarbage("setpause", n)` configurable multiplier — deferred per ADR 0162 §Configurability.
- ❌ Generational nursery (deferred per ADR 0145).
- ❌ Lifting v1 safety mode. Mark phase still pre-paints BLACK; ADR 0187 (DFS + real roots via stack walk) lifts the safety mode.

## Decision

### `src/codegen/tagged.rs`

Add:

```rust
/// ADR 0186 — auto-trigger threshold for `emit_gc_alloc`. When
/// `g_gc_total_bytes + payload_size >= g_gc_threshold`, the
/// allocator inlines `llvm.call @gc_mark()` + `llvm.call
/// @gc_sweep()`. Threshold doubles after each collection (capped
/// at `GC_THRESHOLD_CAP`) to avoid thrashing on large live sets.
pub(crate) const GC_THRESHOLD_INIT: i64 = 1_048_576;       // 1 MiB
pub(crate) const GC_THRESHOLD_CAP:  i64 = 1_073_741_824;   // 1 GiB
```

### Module init (`src/codegen/emit.rs`)

Inside `emit_module_init` after the existing `g_gc_head` / `g_gc_total_bytes` registrations:

```rust
emit_mutable_i64_global(context, module, types, "g_gc_threshold",
    GC_THRESHOLD_INIT, loc);
register_gc_runtime_funcs(context, module, types, loc);
```

`register_gc_runtime_funcs` emits two `llvm.func` definitions. Both are void → void. Body uses an entry block, calls the existing `emit_gc_mark_inline` / `emit_gc_sweep_inline` to populate the walk into that block, then appends `llvm.return`. Reuses `emit_llvm_func` (existing helper at `emit.rs:1734`).

### `emit_gc_alloc` extension (`src/codegen/primitive.rs`)

Before the existing OOM check, insert the threshold gate. Pseudo-Rust outline:

```rust
let total_size = payload_size + GC_HEADER_SIZE;
let bytes_addr = emit_addressof(..., "g_gc_total_bytes", ...);
let cur_total  = emit_load(..., types.i64, ...);
let new_total  = arith::addi(cur_total, total_size, loc);
let thr_addr   = emit_addressof(..., "g_gc_threshold", ...);
let threshold  = emit_load(..., types.i64, ...);
let should_gc  = arith::cmpi(Sge, new_total, threshold, loc);

scf::if (should_gc) {
    emit_libc_call_void("gc_mark", &[]);
    emit_libc_call_void("gc_sweep", &[]);
    // Doubling: refresh post-sweep total + threshold.
    let post_sweep = emit_load(bytes_addr, i64);
    let doubled    = arith::muli(post_sweep, const(2));
    let cap        = const(GC_THRESHOLD_CAP);
    let cur_thr    = emit_load(thr_addr, i64);
    let needs_grow = arith::cmpi(Sgt, doubled, cur_thr, loc);
    scf::if (needs_grow) {
        let capped     = arith::select(cmpi(Sgt, doubled, cap), cap, doubled);
        emit_store(capped, thr_addr);
    }
}
```

Then the existing alloc body (OOM check / header init / `g_gc_total_bytes += total_size`) runs unchanged.

### `Callee::Builtin(CollectGarbage)` arm

Replace the inline calls:

```rust
emit_gc_mark_inline(context, block, types, loc);
emit_gc_sweep_inline(context, block, types, loc);
```

with:

```rust
emit_libc_call_void(context, block, "gc_mark",  &[], loc);
emit_libc_call_void(context, block, "gc_sweep", &[], loc);
```

The before / after total-bytes delta computation is unchanged.

### Tests

`tests/phase3_gc_auto_trigger.rs` (NEW, 3 e2e regression pins):

1. **Threshold-crossing loop**: allocate enough strings to cross 1 MiB (e.g. 1024 strings of 1500 bytes each ≈ 1.5 MiB), then `print("ok")`. Pins that auto-trigger does not crash.
2. **Count consistency after trigger**: snapshot `count` before and after the loop; assert that `after > before` (v1 leak, allocations still accumulate) AND `after` is finite (no underflow corruption).
3. **Explicit `collectgarbage()` post-trigger**: after the loop, call `collectgarbage()`; assert it returns `0` (v1 safety mode unchanged — auto-trigger does not break the explicit-call contract).

All three pass Day 0 if the impl is correct (the auto-trigger fires but in v1 safety mode it is observably a no-op except for the threshold doubling, which is not externally visible).

## Alternatives considered

- **Bundle the threshold global doubling into `gc_sweep` itself.** Rejected — `gc_sweep` is paired with `gc_mark` by both the explicit `collectgarbage()` arm and the auto-trigger; pushing the threshold update into sweep would make explicit calls also double the threshold, which is wrong per ADR 0162 (doubling is for the auto-trigger's "couldn't keep up" feedback, not user-initiated collection). Keeping doubling at the caller is the principled split.
- **Skip the factoring; emit `func.func @gc_mark` instead of `llvm.func`.** Rejected — `func.func` was already migrated off per the comment at `emit.rs:1672`; consistency with `emit_llvm_func` keeps the dialect uniform.
- **Inline the threshold check at every `emit_gc_alloc` callsite, skip the wrapper change.** Rejected — `emit_gc_alloc` is the chokepoint; per-site duplication defeats ADRs 0157/0158.
- **Use a separate `emit_gc_alloc_with_trigger` wrapper, leave the bare `emit_gc_alloc` unchanged.** Rejected — that creates two allocator entry points; the chokepoint property of ADR 0157 is what makes future GC features (mark, sweep, generational) tractable. Single entry, layered behaviour.

## Consequences

**Positive**
- `gc_mark` / `gc_sweep` are first-class module-level functions — visible in `--emit-llvm` debugging, callable from future intrinsics (finalizer dispatch, weak-table cleanup), and addressable from runtime tests.
- The 1 MiB threshold limits worst-case heap growth in v1 (when 0186 lifts safety mode, the leak ceases too).
- Doubling logic prevents thrashing on programs with a large live working set.
- `emit_gc_alloc` is still the single chokepoint — all GC infrastructure remains layerable here.

**Negative**
- Every allocation pays one extra load + cmpi + (almost-always-skipped) scf.if branch. The fast path stays cheap; LLVM constant-folds nothing here but the branch is well-predicted.
- The `llvm.func @gc_mark` / `@gc_sweep` emissions add ~50 LOC of MLIR per module. Cosmetic.

**Locked in until superseded**
- `g_gc_threshold` is the single source of truth for the auto-trigger boundary. Future ADRs (e.g. `setpause` configurability) mutate it through dedicated helpers.
- `GC_THRESHOLD_INIT = 1 MiB` and `GC_THRESHOLD_CAP = 1 GiB` are the v1 constants. Both can be tuned without ABI break.

## Documentation updates

- [x] §8 — adds 0187.
- [x] ADR 0162 status reminder — implementation DONE here.
- [x] ADR 0185 future-work — `func.func` factoring DONE here.

## Test count delta

```
Step 0: 1415 (after bc1bf0f)
C1 (doc): 1415 → 1415
C2 (3 regression pins Red Day 0): 1415 → 1418
C3 (impl): 1418 → 1418 (all green)
```

## Critical files

- `src/codegen/tagged.rs`:
  - Add `GC_THRESHOLD_INIT` / `GC_THRESHOLD_CAP` constants.
- `src/codegen/emit.rs`:
  - Add `register_gc_runtime_funcs` (emits `llvm.func @gc_mark` / `@gc_sweep` using existing inline helpers as body builders).
  - Register `g_gc_threshold` global + call `register_gc_runtime_funcs` from `emit_module_init`.
  - Rewire `Callee::Builtin(CollectGarbage)` 0-arg arm from inline helper calls to `llvm.call`s.
- `src/codegen/primitive.rs`:
  - Extend `emit_gc_alloc` with threshold check + doubling logic at entry.
- `tests/phase3_gc_auto_trigger.rs` (NEW) — 3 e2e regression pins.

## Risks

| Risk | Mitigation |
|---|---|
| `llvm.func @gc_mark` redefinition on `emit_module_unverified` re-entry | `emit_module_init` runs once per module; same lifecycle as other emit helpers. |
| Doubling overflow when `post_sweep_total * 2` exceeds i64 range | `GC_THRESHOLD_CAP` is 1 GiB, so realistic post-sweep totals never approach overflow. Worst case clamp via `arith::select` against the cap. |
| Auto-trigger infinite loop (sweep doesn't free, threshold doesn't grow fast enough) | Doubling logic guarantees threshold grows at least proportional to live set; in v1 safety mode this is exercised on every collection. |
| Auto-trigger fires inside the `gc_mark` / `gc_sweep` walk path itself | The walks call `free` (sweep) and tag stores (mark) — neither allocates. No recursion concern. |
| `func.func` vs `llvm.func` dialect mismatch | Reuse `emit_llvm_func` (existing precedent, `emit.rs:1734`); the codebase migrated off `func.func` per the comment at `emit.rs:1672`. |
| Tests can't distinguish "auto-trigger fires" from "auto-trigger doesn't fire" since v1 safety mode is observably no-op | Tests pin "doesn't crash + count consistent + explicit-call contract unchanged"; the trigger firing is exercised structurally by allocating > 1 MiB but unobservable as freeing. |

## Future work

- ADR 0187 — Worklist + per-type DFS (lifts v1 safety mode, makes auto-trigger actually free).
- ADR 0188 — Stack walk for alloca-slot roots (paired with 0187 for safe freeing).
- `collectgarbage("setpause", n)` builtin to configure the doubling multiplier.
- Tune `GC_THRESHOLD_INIT` based on profiling once real workloads exist.

## References

- [ADR 0157](0157-gc-allocator-wrapper.md) — `emit_gc_alloc` chokepoint extended here.
- [ADR 0161](0161-gc-sweep-phase.md) — sweep algorithm consumed via `@gc_sweep`.
- [ADR 0162](0162-gc-auto-trigger.md) — design implemented here.
- [ADR 0182](0182-param-inference-kind-parameterised-helpers.md) — Tidy First extract-before-second-consumer precedent.
- [ADR 0185](0185-gc-mark-sweep-v1-safety-mode.md) — inline walks factored here.
