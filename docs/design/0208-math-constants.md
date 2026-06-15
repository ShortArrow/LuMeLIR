# 0208. `math.pi` / `math.huge` Constants

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Bucket B item — Lua 5.4 §6.7 lists `math.pi` (the value of π) and `math.huge` (`+inf`) as numeric constants. LuMeLIR's `math` namespace currently dispatches only through `Builtin::from_namespace_method` (the Call-form chokepoint for `math.<method>(args)`). Plain `print(math.pi)` fails with `undefined name 'math'` because `math` is not a HIR-resolved ident; it's only recognised as a namespace prefix in the Call form.

This ADR adds a special-case to HIR's `ExprKind::Index` lowering: when the AST is `Index(Ident("math"), Str(name))` AND `name` is a known math constant AND `math` is otherwise unresolved, emit a Number literal directly.

## Scope (literal)

- ✅ `math.pi` → `std::f64::consts::PI` (≈ 3.141592653589793).
- ✅ `math.huge` → `f64::INFINITY` (prints as `"inf"` per printf `%g`).
- ✅ The math.<name> shape composes with the existing arithmetic / print paths — no Bridge-style ABI change.
- ✅ User shadowing respected: if `math` is a local / function name, the resolver wins and the constant special-case is skipped (per ADR 0103 shadowing precedent).
- ❌ `math.maxinteger` / `math.mininteger` — deferred to the ADR 0196 Integer/Float arc (these require the integer subtype).
- ❌ Other identifier paths (`math.foo` where foo is not pi/huge) — fall through to the existing UndefinedName/namespace-call path, unchanged.

## Decision

`src/hir/mod.rs`:

1. New module-level helper `math_constant_value(name: &str) -> Option<f64>` returning `Some(PI)` for `"pi"`, `Some(INFINITY)` for `"huge"`, else `None`.
2. At the top of the `ExprKind::Index` arm in `lower_expr`, before invoking `self.lower_expr(target)?`, pattern-match on `Index(Ident("math"), Str(name))` where `self.resolve("math").is_none()` and `self.function_names` doesn't contain `"math"`. If `math_constant_value(name)` returns `Some(v)`, return `HirExpr { kind: HirExprKind::Number(v), span: expr.span }`.

Codegen / runtime: no change. The Number literal flows through the existing path.

## Tests

`tests/phase4_math_constants.rs` (NEW, 3 e2e):

1. `print(math.pi)` → `"3.14159"` (printf %g default precision).
2. `print(math.huge)` → `"inf"`.
3. `print(math.pi * 2)` → `"6.28319"` (sanity: composes through `*`).

## Alternatives considered

- **Treat `math` as a synthetic global Table populated with constants at module init.** Rejected — requires runtime allocations + a special-case HIR symbol; the HIR-time short-circuit is leaner.
- **Bundle Integer-subtype constants (`maxinteger`, `mininteger`).** Rejected — those require ADR 0196's Integer arc; bundling here pollutes the constant scope.
- **Use the existing namespace-call dispatch for "non-call" identifier access.** Rejected — the namespace-call chokepoint is specifically Call-form (`Index { Ident(ns), Str(method) }` as the Callee of a `Call`). Reusing it would conflate two different shapes.

## Test count delta

```
Step 0: 1454 (after 9c3cfe5)
C3 (impl): 1454 → 1457
```

## References

- [Lua 5.4 §6.7 math](https://www.lua.org/manual/5.4/manual.html#6.7)
- [ADR 0103](0103-stdlib-string-begin.md) — namespace-call dispatch chokepoint.
- [ADR 0196](0196-integer-float-subtype-design.md) — Integer subtype design (where `maxinteger`/`mininteger` lands).
