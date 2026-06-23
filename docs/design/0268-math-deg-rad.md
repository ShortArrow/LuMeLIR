# 0268. `math.deg(x)` / `math.rad(x)` (N7-7)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::MathDeg` / `Builtin::MathRad`; arity `(1, 1)` each. Param `[Number]`, ret `[Number]`.
- ✅ Codegen: `arith.mulf x, const` where the constant is folded at compile time:
  - `math.deg(x) = x * (180 / π)` (~57.2957795...)
  - `math.rad(x) = x * (π / 180)` (~0.0174532925...)
- ✅ No libm dependency — pure SSA arithmetic.

## Tests

`tests/phase4_n7_math_deg_rad.rs` (3 e2e): `deg(π) == 180`, `rad(180)` ≈ π, `deg(0) == 0`. 1669 → 1672.

## References

- Lua 5.4 §6.7 — math library.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N7.
