# 0262. `math.fmod(x, y)` (N7-1)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

First N7 sub-ADR. Roadmap rebuild (2026-06-21) §N7 lists `math.fmod` among the spec-gap stdlib items. Per Lua 5.4 §6.7, `math.fmod` returns the C-style truncation remainder — distinct from Lua's `%` operator which is floor-mod.

## Scope (literal)

- ✅ New `Builtin::MathFmod` HIR variant. Lookup table entry `"fmod"` (resolved via `math_from_method`). Arity `(2, 2)`. Display name `"math.fmod"`. Param kinds `[Number, Number]`. Ret kinds `[Number]`.
- ✅ Codegen: `libm fmod(f64, f64) -> f64` declared at module init; `Callee::Builtin(MathFmod)` emits a 2-arg libc call.
- ✅ `infer_kind` arm: `Number`.
- ❌ Integer-flavor result for two-integer args (Lua 5.4 returns integer when both are integers). Today's pipeline silently floats — same trade-off as the existing math.* builtins per ADR 0210.

## Tests

`tests/phase4_n7_math_fmod.rs` (NEW, 3 e2e):

1. `math.fmod(7, 3) == 1`.
2. `math.fmod(-7, 3) == -1` (truncation toward zero — verifies the distinction from `%`).
3. `math.fmod(0, 5) == 0`.

## Test count delta

```
Step 0:  1655 (after N3 close)
N7-1:    1655 → 1658
```

## References

- [Lua 5.4 §6.7](https://www.lua.org/manual/5.4/manual.html#6.7) — math library.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N7 stdlib gap closure.
