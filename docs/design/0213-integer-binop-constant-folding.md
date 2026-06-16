# 0213. Integer + Integer BinOp Constant Folding

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Fifth M1 sub-ADR. ADRs 0209-0212 introduced `HirExprKind::Integer(i64)` and made `math.type` / `math.tointeger` / `math.maxinteger` subtype-aware against AST-level Integer literals. But static arithmetic — `math.type(1 + 2)` — still returned `"nil"` because BinOp lowering produced an `Arith` HIR node (Number-kinded) regardless of operand subtype, breaking subtype propagation at the first arithmetic site.

Lua 5.4 §3.4.1: integer-preserving ops on two integer operands keep integer subtype. `+ - * // % & | ~ << >>` between integers produce integer; `/` and `^` always produce float; mixed integer/float produces float.

## Scope (literal)

- ✅ HIR-level constant fold: when both `lhs` and `rhs` lower to `HirExprKind::Integer`, fold integer-preserving ops to `HirExprKind::Integer(value)`.
- ✅ Ops folded: `Add`, `Sub`, `Mul` (with `checked_*` overflow guard), `FloorDiv`, `Mod` (with `b != 0` guard), `BitAnd`, `BitOr`, `BitXor`, `Shl` / `Shr` (with shift in `0..64`).
- ✅ Overflow / division-by-zero / shift-out-of-range falls through to the existing f64 BinOp path (correctness preserved, subtype lost — Phase B silent demotion already documented).
- ❌ Runtime Integer-Integer arithmetic. Two `Local i = 1; Local j = 2; print(math.type(i + j))` still returns `"nil"` — no runtime subtype tracking through slots yet (Phase C scope).
- ❌ `Div` (`/`) folding — always Float per Lua §3.4.1.
- ❌ `Pow` (`^`) folding — always Float per Lua §3.4.1.
- ❌ Unary `-` on Integer (separate `UnaryOp` arm; not yet needed for M1).
- ❌ Mixed Integer/Float folding to Float (Phase B already silently demotes Integer to Number at the leaf; mixed expressions reach the f64 path unchanged).

## Decision

`src/hir/mod.rs`, top of `ExprKind::BinOp` arm:

```rust
if let (HirExprKind::Integer(a), HirExprKind::Integer(b)) =
    (&lhs_hir.kind, &rhs_hir.kind)
{
    let folded: Option<i64> = match op {
        BinOp::Add => a.checked_add(*b),
        BinOp::Sub => a.checked_sub(*b),
        BinOp::Mul => a.checked_mul(*b),
        BinOp::FloorDiv if *b != 0 => Some(a.div_euclid(*b)),
        BinOp::Mod if *b != 0 => Some(a.rem_euclid(*b)),
        BinOp::BitAnd => Some(a & b),
        BinOp::BitOr => Some(a | b),
        BinOp::BitXor => Some(a ^ b),
        BinOp::Shl if (0..64).contains(b) => Some(a << b),
        BinOp::Shr if (0..64).contains(b) => Some(((*a as u64) >> b) as i64),
        _ => None,
    };
    if let Some(v) = folded {
        return Ok(HirExpr { kind: HirExprKind::Integer(v), span: expr.span });
    }
}
```

Overflow on `checked_*` returns `None`, falling through to the existing arith path. Shift uses logical (unsigned) right-shift per Lua 5.4 `>>` semantics.

## Tests

`tests/phase4_integer_binop_folding.rs` (NEW, 6 e2e):

1. `math.type(1 + 2)` → `"integer"`.
2. `math.type(5 - 3)` → `"integer"`.
3. `math.type(4 * 3)` → `"integer"`.
4. `math.tointeger(10 // 3)` → `"3"`.
5. `math.type(7 & 5)` → `"integer"`.
6. `math.type(1 + 2.0)` → not `"integer"` (mixed must not fold).

Four codegen unit tests (`emit_addition_produces_arith_addf`, `emit_subtraction_uses_arith_subf`, `emit_multiplication_uses_arith_mulf`, `emit_modulo_uses_floor_for_lua_semantics`) migrate Integer-literal sources (`1 + 2`, `3 - 1`, …) to fractional operands (`1.5 + 2.5`, …) so the f64 codegen path still receives coverage — Integer-Integer sources now fold at HIR.

## Test count delta

```
Step 0: 1467 (after 0212 land)
C3 (impl + 6 e2e): 1467 → 1473
```

## References

- [Lua 5.4 §3.4.1 — Arithmetic Operators](https://www.lua.org/manual/5.4/manual.html#3.4.1)
- [ADR 0209](0209-integer-ast-hir-variant.md) — `HirExprKind::Integer`.
- [ADR 0210](0210-math-type-static-shape.md) — `math.type` static-shape (constant folding extends its reach).
- [ADR 0211](0211-math-tointeger-static-shape.md) — `math.tointeger` static-shape.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone.
