# 0273. `math.log(x [, base])` (N7-12)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-26
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `Builtin::MathLog` arity widened `(1, 1)` → `(1, 2)`.
- ✅ Param kinds: `[Number, Number]` (positional).
- ✅ Codegen: 1-arg → `libm log(x)`; 2-arg → `log(x) / log(base)` via `arith.divf`.
- ❌ Complex / negative base validation — deferred (libm log returns NaN; Lua spec is silent on negative-base behaviour).
- ❌ Integer-base fast path (e.g. log2 intrinsic) — deferred.

## Tests

3 e2e (`tests/phase4_n7_math_log_base.rs`): natural log unchanged; `log(1000, 10) = 3`; `log(8, 2) = 3`. 1683 → 1686.

## References

- Lua 5.4 §6.7.
- ADR 0270 — N7-9 `math.atan` 2-arg precedent (same arity-widening shape).
