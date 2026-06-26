# 0274. `math.ult(m, n)` (N7-13)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-26
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::MathUlt`; arity `(2, 2)`; params `[Number, Number]`; ret `[Bool]`.
- ✅ Codegen: `arith.fptosi` both Number args to i64, then `arith.cmpi ult` — single op, no libc dependency.
- ❌ Runtime non-integer rejection — current Number model is f64; truncation is silent. Matches Lua 5.4 behaviour when a non-integer is passed (raises "number has no integer representation"), but our impl differs (silent fptosi). Documented divergence; integer-kind enforcement deferred to the Number-subtype work.

## Tests

3 e2e (`tests/phase4_n7_math_ult.rs`): `ult(1, 2) = true`; `ult(5, 2) = false`; `ult(-1, 5) = false` (-1 as u64 is `2^64-1`). 1686 → 1689.

## References

- Lua 5.4 §6.7.
- ADRs 0262-0273 — sibling N7 increments.
