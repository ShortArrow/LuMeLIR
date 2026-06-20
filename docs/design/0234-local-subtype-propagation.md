# 0234. Subtype Propagation Through Locals + Arithmetic

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third M8 sub-ADR. [ADR 0232](0232-local-number-subtype.md) populated `LocalInfo::subtype` from direct literal RHS only — `local x = 42` gets `Integer`, but `local y = x + 1` stayed `Unknown` because the BinOp arm of the classifier was a fall-through to `Unknown`. This ADR widens the classifier so the subtype flows through Local reads + integer-preserving arithmetic.

Lua 5.4 §3.4.1 specifies which ops preserve the integer subtype: `+ - * // % & | ~ << >>` keep Integer when both operands are Integer; `/` and `^` always return Float; mixed Integer/Float operands return Float. The classifier mirrors that table.

## Scope (literal)

- ✅ `classify(HirExpr, &[LocalInfo])` recursively walks:
  - `HirExprKind::Integer(_)` → `Integer`
  - `HirExprKind::Number(_)`  → `Float`
  - `HirExprKind::Local(idx)` → `locals[idx].subtype` (forward whatever the previous statement assigned)
  - `HirExprKind::BinOp { op, lhs, rhs }`:
    - Integer-preserving op AND both operands Integer → `Integer`
    - `Div` / `Pow`            → `Float` (always)
    - Any operand Float        → `Float`
    - Otherwise                → `Unknown`
  - Everything else            → `Unknown`
- ✅ Forward single-pass — by the time the post-pass walks stmt N, any Locals defined at stmt M < N have their subtype recorded. Lua's lexical scoping makes this sufficient: a Local can only be used after its declaration in the same scope.
- ✅ ADR 0233 print/tostring `%lld` dispatch lights up for BinOp-produced Integer-subtype Locals — they print without `.0` artifacts.
- ✅ ADR 0232 merge semantics still apply on reassignment (`x = 1; x = x + 1` keeps Integer; `x = 1; x = x + 1.5` widens to Unknown).
- ❌ UnaryOp `-`. Future micro-extension; `local y = -x` for Integer x is rare relative to BinOp.
- ❌ Call results. Even `string.len(s)` (returns Number / known-integer-shaped) classifies as `Unknown` because the post-pass doesn't introspect builtin return semantics. Future widening keyed on per-Builtin return subtype.
- ❌ ForNumeric loop variable. The induction variable is typed Number but its subtype after the loop depends on start/stop/step shapes — future extension.
- ❌ Fixpoint iteration over backward references inside loops. Single forward pass means `local x = 1; while c do x = x + 1; ... end` keeps x's subtype Integer only because the merge sees an Integer RHS each iteration; that holds for this case but more complex patterns may surface as Unknown.

## Decision

### Classifier transitions

```
Integer + Integer  → Integer   (Add/Sub/Mul/FloorDiv/Mod/Bitwise/Shifts)
Integer / Integer  → Float     (Div is spec-mandated Float)
Integer ^ Integer  → Float     (Pow is spec-mandated Float)
Integer + Float    → Float
Float   + Integer  → Float
Unknown anywhere   → Unknown
```

The classifier is purely structural — no constant evaluation, no operand value inspection. Cost is proportional to expression tree depth at post-pass time.

### Composes with M8-A merge

Each stmt's RHS classification is merged into the LHS Local's existing subtype. So a Local that receives Integer RHS at one stmt and a Float RHS at another widens to Unknown — same rule as M8-A, just with broader RHS coverage.

## Tests

`tests/phase4_m8_subtype_propagation.rs` (NEW, 7 e2e):

1. `local a = 10; local b = a + 5; math.type(b)` → `"integer"`.
2. `local a = 6; local b = 7; local c = a * b; math.type(c)` → `"integer"`.
3. `local a = 10; local b = a / 2; math.type(b)` → `"float"` (Div is always Float).
4. `local a = 10; local b = a // 3; math.type(b)` → `"integer"` (FloorDiv preserves Integer).
5. `local a = 10; local b = a + 1.5; math.type(b)` → `"float"` (mixed widens).
6. `local a = 12; local b = a & 7; math.type(b)` → `"integer"` (BitAnd preserves Integer).
7. Chain of Integer ops then `print(c)` → `"300"` (proves the ADR 0233 `%lld` path lights up for BinOp results).

## Test count delta

```
Step 0:  1550 (after ADR 0233)
C3 (impl + 7 e2e): 1550 → 1557
```

## References

- [ADR 0232](0232-local-number-subtype.md) — classifier foundation.
- [ADR 0233](0233-local-integer-print-tostring.md) — `%lld` consumer that benefits from BinOp-propagated Integer subtype.
- [ADR 0213](0213-integer-binop-constant-folding.md) — Integer+Integer const fold sibling; this ADR extends the same op-table to runtime Locals.
- [Lua 5.4 §3.4.1](https://www.lua.org/manual/5.4/manual.html#3.4.1) — arithmetic operator type rules.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M8 milestone.
