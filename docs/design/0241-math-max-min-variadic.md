# 0241. `math.max` / `math.min` — Variadic Reduce

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M11 sub-ADR. Lua 5.4 §6.7 `math.max(x, ...)` and `math.min(x, ...)` accept 1 or more Number arguments and return the largest / smallest. The variadic dispatch shape (arity 1+, every position Number) is the same as `string.char` (ADR 0113). The reduce body picks one of the MLIR `arith.maximumf` / `arith.minimumf` ops.

## Scope (literal)

- ✅ Two new HIR `Builtin` variants: `MathMax`, `MathMin`.
- ✅ `math_from_method` recognises `"max"` / `"min"`.
- ✅ `arity` `(1, usize::MAX)`; `ret_kinds` `[Number]`; `param_kinds_for_arity` `[Number]` (single-position slice); `expected_param_kind` returns `Number` for every position; `infer_kind` → `Number`.
- ✅ Codegen emits a left-to-right reduce: lower `args[0]` as the accumulator, then for each subsequent arg lower it and replace the accumulator with `arith.maximumf(acc, next)` (or `arith.minimumf`).
- ✅ Single-argument call is the identity (no reduce iteration).
- ❌ Subtype-aware Integer-Integer max/min via `arith.maxsi` / `arith.minsi`. Phase B silent demotion stores Number-kind Locals as f64; the float reduce path produces correct results for integer-shaped values within ±2^53. Future M8-extended ADR can short-circuit to the integer ops when all args have Integer subtype.
- ❌ Mixed Number / String args (Lua's `__lt` metamethod chain for non-Number args). The HIR arg-kind validation rejects non-Number args before codegen.
- ❌ Empty-args form. Lua spec says `math.max()` / `math.min()` with no args is a runtime error; the HIR `arity` rejection (arity ≥ 1) covers this at compile time.

## Decision

### `expected_param_kind` joins `StringChar`

`StringChar` is the existing precedent for "every argc has every position kind = Number". `MathMax` / `MathMin` join the same arm so the `lower_namespace_builtin_call` arg validation loop iterates correctly for any argc.

### Reduce shape

```text
acc = lower(args[0])
for next in args[1..]:
    next_val = lower(next)
    acc = arith.maximumf(acc, next_val)   # or minimumf
yield acc
```

MLIR's `arith.maximumf` / `arith.minimumf` follow IEEE 754-2008 maxNum / minNum behaviour — propagate NaN if either operand is NaN. Lua spec doesn't pin NaN semantics for `math.max` / `math.min` so this is acceptable.

## Tests

`tests/phase4_m11b_math_max_min.rs` (NEW, 7 e2e):

1. `math.max(3, 7)` → `7`.
2. `math.min(3, 7)` → `3`.
3. `math.max(42)` → `42` (1-arg identity).
4. `math.max(1, 5, 3, 9)` → `9` (4-arg left-to-right reduce).
5. `math.min(-2, -5, -1)` → `-5`.
6. `math.max(3.2, 3.7, 3.5)` → `3.7`.
7. `math.max(math.abs(-3), math.sqrt(4))` → `3` (composes with other math.* builtins).

## Test count delta

```
Step 0:  1587 (after ADR 0240)
C3 (impl + 7 e2e): 1587 → 1594
```

## References

- [ADR 0102](0102-phase2-7q-stdlib-math-continuation.md) — sibling math.* dispatch shape.
- [ADR 0113](0113-phase2-7v-stdlib-string-char.md) — `expected_param_kind` variadic-Number precedent (StringChar).
- [Lua 5.4 §6.7 `math.max` / `math.min`](https://www.lua.org/manual/5.4/manual.html#pdf-math.max).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M11 milestone.
