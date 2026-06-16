# 0212. `math.maxinteger` / `math.mininteger` Integer Constants

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Fourth M1 sub-ADR. Integer-kind parallels to ADR 0208's `math.pi` / `math.huge` Number constants.

Lua 5.4 §6.7:
- `math.maxinteger` = largest integer (= `i64::MAX` in standard Lua).
- `math.mininteger` = smallest integer (= `i64::MIN`).

These were marked deferred in ADR 0208 pending the Integer subtype machinery. After ADRs 0209 + 0210 + 0211 introduced `HirExprKind::Integer` and static-shape subtype distinction, the constants compose naturally: lower to `HirExprKind::Integer(i64::MAX/MIN)`, and `math.type(math.maxinteger)` already returns `"integer"` via the ADR 0210 emit arm.

## Scope (literal)

- ✅ HIR helper `math_integer_constant(name) -> Option<i64>` returning `i64::MAX` for `"maxinteger"`, `i64::MIN` for `"mininteger"`.
- ✅ HIR `ExprKind::Index` lowering arm checks `math_integer_constant` after the existing `math_constant_value` (Number) check. On match, emits `HirExprKind::Integer(value)`.
- ✅ `math.type(math.maxinteger)` returns `"integer"` (composes with ADR 0210).
- ❌ Subtype-distinguished print preserving precision. Phase B demotes to f64 → loses precision near `i64::MAX`; `print(math.maxinteger)` outputs `9.22337e+18` not `9223372036854775807`. Phase C scope (ADR 0214+ candidate).
- ❌ Integer arithmetic with these constants. Same Phase B demotion limitation.

## Decision

`src/hir/mod.rs`:

```rust
fn math_integer_constant(name: &str) -> Option<i64> {
    match name {
        "maxinteger" => Some(i64::MAX),
        "mininteger" => Some(i64::MIN),
        _ => None,
    }
}
```

The `ExprKind::Index` lowering arm's existing math-constant short-circuit (ADR 0208) extends to call `math_integer_constant` when the Number-kind check returns None. On match, emit `HirExprKind::Integer(value)` (not `Number(value as f64)`).

Codegen and runtime: no change. The existing `HirExprKind::Integer` emit arm (ADR 0209) handles printing via sitofp demotion.

## Tests

`tests/phase4_math_integer_constants.rs` (NEW, 3 e2e):

1. `print(math.type(math.maxinteger))` → `"integer"`.
2. `print(math.type(math.mininteger))` → `"integer"`.
3. `print(math.tointeger(math.maxinteger))` — Phase B demotion produces a Number (not Nil). The exact precision is f64-bounded; test assertion is "not nil" rather than an exact value.

## Test count delta

```
Step 0: 1464 (after 51a09cd)
C3 (impl + 3 e2e): 1464 → 1467
```

## References

- [Lua 5.4 §6.7 math.maxinteger](https://www.lua.org/manual/5.4/manual.html#pdf-math.maxinteger) / [math.mininteger](https://www.lua.org/manual/5.4/manual.html#pdf-math.mininteger)
- [ADR 0208](0208-math-constants.md) — Number-kind `math.pi` / `math.huge`; precedent.
- [ADR 0209](0209-integer-ast-hir-variant.md) — `HirExprKind::Integer`.
- [ADR 0210](0210-math-type-static-shape.md) — `math.type` static-shape.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone.
