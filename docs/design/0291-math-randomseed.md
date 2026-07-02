# 0291. `math.randomseed(seed)` (N7-20)

- **Status:** Accepted (single-arg; Lua 5.4 no-arg secure form deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::MathRandomSeed`; arity `(1, 1)`; param `[Number]`; ret `[]` (void).
- ✅ Codegen calls libc `srand(seed as u32)` via a new extern.
- ✅ `infer_kind` returns `Number` as a Print-precedent placeholder so the call is usable in expression position (returns `0.0`).
- ❌ Lua 5.4 no-arg form (`math.randomseed()` returns `(x1, x2)` from a secure seed source) — deferred.
- ❌ Multi-return form of the 1-arg call — Lua 5.4 also lets the 1-arg form take `(seed[, seed2])`. Deferred.

## Tests

3 e2e (`tests/phase4_n7_20_math_randomseed.rs`): call is stable + runs; same seed → same random sequence (determinism); different seeds → sequences differ (spot check). 1780 → 1783.

## References

- Lua 5.4 §6.7 — `math.randomseed`.
- ADR 0263 — `math.random` sibling (both share `rand`/`srand` from libc).
- Roadmap 2026-07-02 — layer-based sizing (runtime + codegen-arm).
