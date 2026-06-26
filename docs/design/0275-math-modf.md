# 0275. `math.modf(x)` integer part — single-result (N7-14)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-26
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::MathModf`; arity `(1, 1)`; param `[Number]`; ret `[Number]`.
- ✅ Codegen: dispatches libm `trunc(x)` — added to the unary libm extern list.
- ✅ Single-result position: returns integer part only. `print(math.modf(3.75))` → `3`.
- ❌ Multi-return form `(int, frac)` — deferred. Single-assign truncation matches the `string.find` precedent (ADR 0228 / 0229).

## Tests

3 e2e (`tests/phase4_n7_math_modf.rs`): positive (3.75 → 3), negative trunc-toward-zero (-2.6 → -2), integer pass-through (7 → 7). 1689 → 1692.

## References

- Lua 5.4 §6.7.
- ADR 0228 — `string.find` single-result truncation precedent.
- ADRs 0262-0274 — sibling N7 increments.
