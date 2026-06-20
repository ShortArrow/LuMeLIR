# 0232. Runtime Number-Subtype Tracking via `LocalInfo::subtype`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M8 sub-ADR. [M1 ADRs 0209-0214](0209-integer-ast-hir-variant.md) shipped Phase B Integer/Float subtype distinction at the HIR static-shape layer: `math.type(1)` returns `"integer"`, but `math.type(local_var_holding_1)` returns `"nil"` because the subtype is lost the moment a Number literal is bound to a Local. The M1 sub-ADRs explicitly deferred runtime subtype propagation to M8.

This ADR lands the first piece of that runtime tracking: a new `LocalInfo::subtype: NumberSubtype` field populated by a post-pass over the HIR. `math.type(Local)` consumes it to return `"integer"` / `"float"` / `nil` based on the merge of all LocalInit + Assign RHS shapes for that Local.

## Scope (literal)

- ✅ New enum `NumberSubtype { Unknown, Integer, Float }` in `src/hir/ir.rs`.
- ✅ New field `LocalInfo::subtype: NumberSubtype` initialised to `Unknown` at every Local declaration site.
- ✅ Post-pass `propagate_number_subtype(stmts, locals)` at the end of `pub fn lower`. Walks every LocalInit / Assign; classifies the RHS via:
  - `HirExprKind::Integer(_)` → `Integer`
  - `HirExprKind::Number(_)`  → `Float`
  - anything else             → `Unknown`
  Merges with the existing slot subtype via "first seen wins, mismatched seen widens to Unknown".
- ✅ The walker recurses into If (then / elifs / else) / While / Repeat / ForNumeric bodies so reassignments inside control flow update the same Local.
- ✅ `math.type(arg)` emit arm checks `LocalInfo::subtype` when `arg.kind == HirExprKind::Local` and the Local kind is Number. Returns `"integer"` / `"float"` / `"nil"` accordingly.
- ✅ Recurses through user-function bodies — `propagate_number_subtype` runs for each `HirFunction.body` + `HirFunction.locals`.
- ❌ Subtype propagation through Call / BinOp / UnaryOp result Locals. Future M8 sub-ADR (subtype inference for results of `+ - * // % & | ~ << >>` on Integer operands).
- ❌ Subtype-aware `tostring(Local-Integer)`. Future M8-B sub-ADR (precision-preserving `%lld` print).
- ❌ Subtype-aware `io.write(Local-Integer)`. Future M8-C.
- ❌ Phase C i64 arithmetic ops at runtime. Future M8-D.
- ❌ Integer comparison `== < <= ` short-circuit. Future M8-E.
- ❌ Subtype propagation across function boundaries (caller's Integer arg into callee's parameter slot). Future ADR.
- ❌ ForGeneric / ForIpairs / ForPairs body walk. Limited demand; future widening.

## Decision

### Merge semantics

```text
merge(Unknown, X)        = X
merge(X, X)              = X
merge(Integer, Float)    = Unknown
merge(Float, Integer)    = Unknown
merge(X, Unknown)        = Unknown    // any future Assign weakens
```

Conservative on widening — the first non-matching RHS resets the Local to `Unknown` for the rest of analysis. Re-narrowing (after a `Unknown` reassignment, all subsequent assigns are Integer) is intentionally NOT supported because the pass is single-shot, post-lower; revisiting per-block would require a fixpoint analysis disproportionate to the M8 minimum-viable scope.

### `math.type(Local)` dispatch

```rust
let local_subtype = if let HirExprKind::Local(LocalId(idx)) = &arg_expr.kind {
    if matches!(locals[*idx].kind, ValueKind::Number) {
        Some(locals[*idx].subtype)
    } else { None }
} else { None };
match (&arg_expr.kind, local_subtype) {
    (HirExprKind::Integer(_), _)           => store "integer"
    (HirExprKind::Number(_), _)            => store "float"
    (_, Some(NumberSubtype::Integer))      => store "integer"
    (_, Some(NumberSubtype::Float))        => store "float"
    _                                      => store Nil
}
```

The static-literal arms keep their ADR 0210 behaviour (highest priority). The Local-with-known-subtype arms light up the new runtime tracking. Everything else falls through to Nil — matching ADR 0210 §"Other shapes (Local, Call result, BinOp, etc.) → TAG_NIL" for the cases the post-pass cannot resolve.

## Tests

`tests/phase4_m8_local_subtype.rs` (NEW, 5 e2e):

1. `local x = 42; print(math.type(x))` → `"integer"`.
2. `local x = 3.14; print(math.type(x))` → `"float"`.
3. `local x = tonumber("5"); print(math.type(x))` → `"nil"` (Call RHS → Unknown).
4. `local x = 1; x = 2.5; print(math.type(x))` → `"nil"` (merge Integer + Float = Unknown).
5. `local x = 1; x = 2; x = 3; print(math.type(x))` → `"integer"` (consistent merges stay).

`tests/phase4_math_type.rs` (UPDATED, 1 e2e): the previous Phase B pin `math_type_local_returns_nil_phase_b` is renamed `math_type_local_returns_integer_after_m8a` and expects `"integer"`.

## Test count delta

```
Step 0:  1540 (after ADR 0231)
C3 (impl + 5 new e2e + 1 updated): 1540 → 1545
```

## References

- [ADR 0209](0209-integer-ast-hir-variant.md) — Integer AST + HIR variant (M1 entry).
- [ADR 0210](0210-math-type-static-shape.md) — `math.type` static-shape; this ADR's runtime extension.
- [ADR 0211](0211-math-tointeger-static-shape.md) — `math.tointeger` static-shape (parallel runtime extension is M8-stretch).
- [ADR 0213](0213-integer-binop-constant-folding.md) — BinOp constant fold; future ADR will route Integer-result Locals through the same subtype field.
- [Lua 5.4 §2.1](https://www.lua.org/manual/5.4/manual.html#2.1) — Number subtype semantics.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M8 milestone.
