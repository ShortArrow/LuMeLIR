# 0222. `_G` Extended Surface — Heterogeneous Values + Iteration

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M4 sub-ADR. [ADR 0221](0221-env-globals-chunk-table.md) landed `_G` and `_ENV` as a chunk-level Table via an HIR pre-pass. This ADR pins the broader behavioural surface that composes with the existing Table machinery — heterogeneous Number/String values, `pairs(_G)` iteration, single-level user-fn read+write — and documents the runtime constraints that exist today (homogeneous-typed Table value rejection for Bool/Nil; multi-hop nested-closure capture).

No new HIR or codegen change; this is a feature-coverage decision plus a Red Day 0 pin set so future refactors don't regress the existing surface.

## Scope (literal)

- ✅ Pin: String values in `_G[k]` (`_G.name = "world"`).
- ✅ Pin: Mixed Number / String values across distinct `_G` keys.
- ✅ Pin: `pairs(_G)` iteration counts every key inserted.
- ✅ Pin: Single-level user fn captures `_G` via ADR 0083 upvalue mechanism and can read+write.
- ✅ Pin: Reassignment of an existing `_G[k]` overwrites the previous value.
- ❌ Bool / Nil values stored at a `_G[k]` slot. Current Table machinery rejects with `"table value type mismatch"` at runtime — homogeneous-kind Tables widen only to TaggedValue on String/Number/Function/Table-mixed scenarios. Future ADR widens the Table value-kind constraint.
- ❌ Multi-hop nested closure capture (`outer()` containing `inner()` containing `_G[...]`). Current ADR 0083 closure-cell mechanism rejects `Table`-kind upvalues across two-hop captures. Future ADR (alongside M3-extended Table DFS) lifts this.
- ❌ Free-name fallback. `x = 1` still goes through ADR 0048 auto-declared locals, not `_G.x = 1`.
- ❌ Pre-population of `_G` with builtins (`_G.print`, `_G.string`). Builtins remain HIR early-bind.

## Decision

Behaviour is pinned by 5 e2e tests in `tests/phase4_env_globals_extended.rs`. The pre-pass from ADR 0221 + the existing Table machinery deliver the entire pinned surface without further codegen change.

The runtime restriction on Bool/Nil Table values surfaces as a clear error message (`"table value type mismatch"`); programs that need mixed-tag storage in `_G` can today use only Number/String/Function/Table values. The roadmap note for the future widening lives in §References (M3-extended Table DFS).

## Tests

`tests/phase4_env_globals_extended.rs` (NEW, 5 e2e):

1. `_G.name = "world"; print("hello " .. _G.name)` → `"hello world"`.
2. Mixed `_G.n = 42; _G.s = "hi"; print(_G.n); print(_G.s)` → `"42\nhi"`.
3. `pairs(_G)` iteration over 3 inserted keys counts to 3.
4. Single-level fn read+modify+write composes (counter idiom).
5. `_G.x` reassignment overwrites; final read wins.

## Test count delta

```
Step 0:  1498 (after ADR 0221)
C3 (5 e2e): 1498 → 1503
```

## References

- [ADR 0221](0221-env-globals-chunk-table.md) — `_G` / `_ENV` chunk-level Table foundation.
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure upvalue mechanism used by single-level fn capture.
- [ADR 0080](0080-phase2-8e-iter-pairs.md) — `pairs()` iteration.
- [ADR 0220](0220-gc-tagged-value-roots.md) — TaggedValue chunk-slot roots (sibling foundation for Tables-as-values).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M4 milestone.
