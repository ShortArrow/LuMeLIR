# 0263. `math.random([m[, n]])` (N7-2)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Second N7 sub-ADR. Lua 5.4 §6.7's `math.random` has three arities:
- `math.random()` → float in `[0, 1)`.
- `math.random(n)` → integer in `[1, n]`.
- `math.random(m, n)` → integer in `[m, n]`.

This ADR lands all three via libc `rand()`. `math.randomseed` (deterministic seeding) is a focused follow-up.

## Scope (literal)

- ✅ New `Builtin::MathRandom` HIR variant; lookup via `math_from_method`. Arity `(0, 2)`. Param kinds slice = `[Number, Number]` with the arity bounds enforcing 0/1/2-arg calls. Ret kind `[Number]`.
- ✅ Codegen registers libc `rand() -> i32` at module init. The arm dispatches on `args.len()`:
  - 0 args: `sitofp(rand()) / 2147483647.0` → f64 in `[0, 1)` on glibc.
  - 1 arg: `fmod(rand_f64, n) + 1.0` → in `[1, n]`.
  - 2 args: `m + fmod(rand_f64, n - m + 1)` → in `[m, n]`.
- ✅ `infer_kind` returns `Number` regardless of arity.
- ❌ **`math.randomseed(seed)`** — deferred. Today's runs are non-deterministic per Lua spec (the spec says "the generator is initialized with a deterministic seed when the program starts", but quality / determinism is implementation-defined). A focused N7 follow-up will land `srand` registration + an explicit seed argument.
- ❌ **Integer-flavor result** — same `ValueKind::Number` simplification as the rest of math.*. Lua 5.4 returns integer for the 1-/2-arg forms; today's pipeline keeps them as f64. ADR 0210 carries the cleanup plan.
- ❌ **High-quality PRNG** — Lua 5.4 ships xoshiro256\*\*. libc `rand()` is weaker but adequate for the test surface; a future ADR can substitute a userland xoshiro implementation behind the same Builtin.

## Tests

`tests/phase4_n7_math_random.rs` (NEW, 3 e2e):

1. `math.random() >= 0 and < 1`.
2. `math.random(10)` is in `[1, 10]`.
3. `math.random(5, 8)` is in `[5, 8]`.

## Test count delta

```
Step 0:  1658 (after ADR 0262)
N7-2:    1658 → 1661
```

## References

- [Lua 5.4 §6.7](https://www.lua.org/manual/5.4/manual.html#6.7) — math library; `random` / `randomseed` semantics.
- [ADR 0262](0262-math-fmod.md) — sibling N7-1.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N7 in the N1-N10 path.
