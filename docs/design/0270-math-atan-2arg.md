# 0270. `math.atan(y[, x])` 2-arg form (N7-9)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `Builtin::MathAtan` arity widened from `(1, 1)` to `(1, 2)`.
- ✅ Param kinds slice now `[Number, Number]` (the arity bounds gate the optional 2nd arg).
- ✅ Codegen: split MathAtan out of the unary-libm group; new arm dispatches:
  - 1-arg: `libm atan(y)`.
  - 2-arg: `libm atan2(y, x)` — declared at module init.
- ✅ Replaces the deprecated `math.atan2` per Lua 5.4 §6.7.

## Tests

3 e2e: 1-arg unchanged (`atan(1) ≈ π/4`), 2-arg (`atan(1, 0) ≈ π/2`), quadrant (`atan(-1, -1) ≈ -3π/4`). 1675 → 1678.

## References

- Lua 5.4 §6.7 — math.atan superseded math.atan2.
- ADRs 0262-0269 — sibling N7 increments.
