# 0211. `math.tointeger(x)` — Static-Shape Integer-Valued Check

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Third M1 sub-ADR. Paired with ADR 0210's `math.type`. Lua 5.4 §6.7 `math.tointeger(x)`:
- Returns the integer form of `x` if `x` represents an integer exactly.
- Returns `nil` otherwise.

Same Phase B limitation as ADR 0210: subtype info is lost once a value flows into a Local or BinOp result. This ADR delivers the statically-derivable cases (literal arguments).

## Scope (literal)

- ✅ New `Builtin::MathToInteger` variant.
- ✅ Codegen pattern-matches `args[0].kind`:
  - `HirExprKind::Integer(i)` → write `Number(i as f64)` to the tagged slot.
  - `HirExprKind::Number(n)` where `n.is_finite() && n.fract() == 0.0` → write `Number(n)`.
  - Else → write `Nil`.
- ✅ Returns TaggedValue tmp slot (Number-or-Nil); compatible with `print` consumer per IoRead / GetMetatable precedent.
- ❌ Subtype-aware Local tracking. ADR 0212+ scope.
- ❌ Out-of-range float (`math.tointeger(1e20)`). Today's check rejects via `is_finite()` for `±inf` / `NaN`; values like `1e20` whose `fract()` is `0.0` but exceed `i64::MAX` are still accepted at Phase B (Lua spec rejects). Future ADR tightens.

## Decision

`src/hir/ir.rs`:
- `Builtin::MathToInteger` variant.
- `math_from_method("tointeger")` returns it.
- Arity `(1, 1)`; name `"math.tointeger"`; ret_kinds `[TaggedValue]`; param_kinds_for_arity `[Number]`.

`src/hir/mod.rs::infer_kind`: returns `TaggedValue`.

`src/codegen/emit.rs`: new emit arm parallel to `MathType`, allocates a TaggedValue tmp slot, writes Number for literals where the value is integer-representable, Nil otherwise.

## Tests

`tests/phase4_math_tointeger.rs` (NEW, 4 e2e):

1. `math.tointeger(42)` → `"42"`.
2. `math.tointeger(42.0)` → `"42"` (integer-valued float).
3. `math.tointeger(42.5)` → `"nil"` (fractional).
4. `local x = 42; math.tointeger(x)` → `"nil"` (Phase B limitation pin).

## Test count delta

```
Step 0: 1460 (after 1dc1a8d)
C3 (impl + 4 e2e): 1460 → 1464
```

## References

- [Lua 5.4 §6.7 math.tointeger](https://www.lua.org/manual/5.4/manual.html#pdf-math.tointeger)
- [ADR 0210](0210-math-type-static-shape.md) — paired `math.type` function.
- [ADR 0196](0196-integer-float-subtype-design.md) — Integer/Float subtype design entry.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone.
