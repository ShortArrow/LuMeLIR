# 0145. GC Strategy — Phase 2 Leak, Phase 3 Trigger

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0133](0133-phase2-completion-criteria.md)'s deferral table requires a GC strategy decision **before the first heap-allocated metatable** lands. Tier 1 / Tier 2 metatables work (ADRs 0134 – 0144) used the **existing** table-header `metatable_ptr` slot at offset 32 (ADR 0134's one-time 32→40-byte header growth) — no new heap.

The trigger has therefore **not fired**. But:

- `_ENV` / true globals (per ADR 0133) will need a shared global-environment table, potentially long-lived.
- `__index = Function` and `__call` (deferred) will introduce closure-cell sharing where the same `Function` reaches multiple call sites, so the lifetime question becomes ambient.
- Pattern engine / coroutines (Phase 3) bring their own heap shapes.

A decision recorded now closes the ADR 0133 row and lets follow-up ADRs cite a known plan instead of re-litigating.

## Decision

**Phase 2 keeps the existing leak-everything strategy. Phase 3 will introduce a non-moving mark-and-sweep collector triggered when total heap > 1 MB (or first explicit `collectgarbage()` call).**

Concretely:

1. **Phase 2 = leak.** Every `malloc` (table headers, hash buffers, string objects, closure cells, scratch buffers) lives until process exit. Documented as an explicit policy, not an accident.
2. **Phase 3 = mark-and-sweep.** When Phase 3 starts, a non-moving collector becomes the default. Allocator wraps `malloc` so the collector can walk a per-type free list at sweep time. Roots: thread-local Lua stack, MLIR-emitted local slots (alloca'd), module-level globals.
3. **Trigger for Phase 3 work**: any of (a) `collectgarbage` Lua builtin requested, (b) `_ENV` / global table introduced, (c) coroutine ABI lands, (d) explicit user benchmark RSS > 1 MB. Whichever comes first.
4. **No reference counting, no generational, no incremental.** Phase 3 will reconsider after the simple mark-and-sweep ships.

### Why these choices

- **Leak in Phase 2** is what we already do; making it official prevents a future ADR from accidentally "fixing" the leak with a different policy.
- **Mark-and-sweep** matches the Lua reference implementation's default; existing literature on Lua-specific tricks (write barriers, weak tables) all assume this baseline.
- **Non-moving** keeps the `!llvm.ptr` slot layout stable. A moving collector would require write barriers at every `IndexAssign` / closure-cell write, blowing up codegen.
- **1 MB trigger** is large enough that benchmarks under that size pay zero GC cost; smaller programs run as Phase 2 does today.

## Alternatives considered

- **Reference counting now (Phase 2).** Rejected — every `IndexAssign` would need refcount adjustment; cyclic structures (a table referring to its own metatable) silently leak. Mark-and-sweep handles cycles natively.
- **Generational GC.** Rejected for the first cut — adds nursery/old-gen bookkeeping with no proven win at the scales we benchmark. Phase 3 may revisit after profiling.
- **Incremental / stop-the-world choice.** Deferred to Phase 3 ADR. Initial implementation will be stop-the-world; if pause times become a problem, the same `malloc` wrapper can support incremental marking.
- **Boehm GC (`libgc`) as a dependency.** Rejected — adds a non-LLVM dependency and conservative scanning across MLIR-emitted stack frames is fragile.
- **Skip the GC ADR until the trigger fires.** Rejected per ADR 0133 deferral contract: "before the first heap-allocated metatable". The closer follow-up ADRs (`_ENV`, `__call`) need a plan to cite.

## Consequences

**Positive**
- Phase 2 close criterion (per ADR 0133) — GC row is now satisfied.
- Future ADRs (`_ENV`, `__call`, `__index = Function`, coroutines, pattern engine) inherit the decision; no re-litigation.
- `__metatable` field hiding (ADR 0140) and shared-metatable patterns can be designed against a known collection model.

**Negative**
- Phase 2 binaries leak ALL allocations. Documented limitation. Phase 3 work is now in scope for Phase 3 close.
- "Mark-and-sweep" is a non-trivial implementation. The next time the trigger fires, that ADR is a meaningful unit of work (estimated M/L per Codex sizing).

**Locked in until superseded**
- Non-moving mark-and-sweep as the default Phase 3 strategy.
- 1 MB trigger threshold (revisable in the implementation ADR; the literal value matters less than the order-of-magnitude).
- No write barriers in Phase 2 codegen.

## Documentation updates

- [x] §1–§5 — **no change** (decision-only ADR; no runtime ABI shift).
- [x] §4 LIC — new resolved entry `LIC-gc-strategy-1`.
- [x] §7 open questions — closes "GC strategy" item; opens "Phase 3 GC implementation" as a Phase-3-bucketed future item.
- [x] §8 ADR index — adds 0145.

## Test count delta

```
Step 0:   1330 (after ADR 0144)
C1 (this ADR + SoT):  1330 → 1330 (docs only — decision ADR, no impl)
```

Decision-only ADR per ADR 0133's "GC strategy decision-only" plan (Path α in `plans/zany-giggling-cat.md`).

## Critical files

- `docs/design/0145-gc-strategy.md` (this file).
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.
- `docs/design/0133-phase2-completion-criteria.md` — GC row gets resolved annotation.
- `docs/design/README.md` — index entry.

## Risks

| Risk | Mitigation |
|---|---|
| The 1 MB threshold turns out to be wrong for real workloads | Re-tunable in the Phase 3 implementation ADR; this ADR pins only the order of magnitude. |
| A future Phase 2 ADR accidentally introduces heap-alloc that doesn't show up in the 1 MB benchmark | The trigger conditions explicitly include "user benchmark RSS > 1 MB" — any reproducible regression starts the Phase 3 work. |
| Mark-and-sweep ends up too slow | Phase 3 ADR can revisit with generational/incremental on top; this ADR doesn't preclude that. |
| Write barriers needed earlier than Phase 3 | If a Phase 2 ADR genuinely needs them (e.g. a weak-key table extension), that ADR carries the cost and pre-empts this one. |

## Future work

- **Phase 3 ADR — mark-and-sweep implementation.** Walks: type-tagged allocator wrapper, root set (thread-local stack + alloca slots + globals), mark phase (DFS through `metatable_ptr`, hash entries, array elements, closure-cell upvalues), sweep phase (free list per size class), `collectgarbage` Lua builtin.
- Weak-key / weak-value tables (per Lua spec §2.5.2) — separate ADR atop the mark-and-sweep base.
- Finalizers (`__gc` metamethod) — separate ADR.
- Pause-time tuning (incremental marking, generational) — only if profiling shows need.

## References

- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table row this ADR closes.
- [ADR 0134](0134-metatables-index-read.md) — table header growth (no new alloc, used the existing slot).
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cells (current leak source for capturing closures).
- [ADR 0112](0112-phase2-string-abi-refactor.md) — string-object alloc (current leak source for all strings).
- Lua 5.4 reference manual §2.5 — garbage collection (mark-and-sweep + incremental + generational).
- Lua 5.4 reference manual §2.5.2 — weak tables.
