# 0235. `math.tointeger(Local-Integer)` Returns the Value

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Fourth (closing) M8 sub-ADR. [ADR 0211](0211-math-tointeger-static-shape.md) shipped `math.tointeger` for static literal args; the Phase B limitation pinned `math.tointeger(Local)` to `nil`. M8's runtime subtype tracking (ADRs 0232 + 0234) makes the success path available: a Local with Integer subtype carries an integer-valued f64 and the emit arm can pass it straight through.

This ADR adds the Local arm symmetric to the static `HirExprKind::Integer` arm — read the slot's f64, store it as a TaggedValue Number. Composes with ADR 0234 BinOp propagation, so `local b = a * 7; math.tointeger(b)` returns the integer value.

## Scope (literal)

- ✅ `math.tointeger` emit arm gains a Local-Integer pattern: `HirExprKind::Local(idx)` where `locals[idx].kind == Number` AND `locals[idx].subtype == NumberSubtype::Integer` → load slot f64 + store as TaggedValue Number (the success path).
- ✅ All other Local subtype states (`Float`, `Unknown`) fall through to the existing `_ → Nil` arm.
- ✅ Composes with M8-C: BinOp-produced Integer-subtype Locals route through this arm too.
- ❌ Runtime fractional check for Float / Unknown-subtype Locals. `math.tointeger(x)` per Lua spec should runtime-check `x.fract() == 0.0` and return Nil only on failure. Static-shape arms (the existing Integer / Number literal + this new Local-Integer) cover the common case; runtime arithmetic-check arm for Unknown subtype is deferred.
- ❌ Subtype-aware Integer comparison (`local i = 1; if i == 1 then ...`). Comparison already works because the cmpf path handles equal f64 values; subtype-aware narrowing (Integer == Integer dispatches via cmpi) is a future micro-extension.
- ❌ `math.fmod` / `math.modf` / other math fns benefiting from Integer-subtype dispatch. Future opportunistic widening.

## Decision

The new arm slots between the existing `HirExprKind::Number` (fractional check) arm and the fallthrough Nil arm:

```rust
match &arg_expr.kind {
    HirExprKind::Integer(i) => { /* existing — Number(i as f64) */ }
    HirExprKind::Number(n) if n.is_finite() && n.fract() == 0.0 => { /* existing */ }
    HirExprKind::Local(idx)
        if locals[idx].kind == Number && locals[idx].subtype == Integer
        => { /* NEW: load + store as Number */ }
    _ => { /* existing — Nil */ }
}
```

The Local subtype arm uses `emit_load` directly (the slot stores f64) and `emit_value_slot_store_number` — same writer as the literal Integer arm. No runtime check needed: the M8-A/C post-pass guarantees the slot holds an integer-valued f64 at every observable assignment.

## Tests

`tests/phase4_m8_math_tointeger_local.rs` (NEW, 4 e2e):

1. `local x = 42; math.tointeger(x)` → `"42"`.
2. `local a = 5; local b = a * 7; math.tointeger(b)` → `"35"` (BinOp propagation).
3. `local x = 3.14; math.tointeger(x)` → `"nil"` (Float subtype).
4. `local x = tonumber("7"); math.tointeger(x)` → `"nil"` (Unknown subtype).

`tests/phase4_math_tointeger.rs` (UPDATED, 1 e2e): the Phase B pin `math_tointeger_local_returns_nil_phase_b` becomes `math_tointeger_local_returns_integer_after_m8d` and expects `"42"`.

## Test count delta

```
Step 0:  1557 (after ADR 0234)
C3 (impl + 4 new e2e + 1 updated): 1557 → 1561
```

## M8 milestone close

ADRs 0232 + 0233 + 0234 + 0235 land the M8 minimum-viable close at 4/4-6 sub-ADRs. Remaining M8-stretch work:

- **Phase C i64 slots** — store Integer-subtype Locals as i64 instead of f64; lifts precision past ±2^53.
- **`io.write(Local-Integer)`** — sibling consumer.
- **Subtype-aware concat** for Local-Integer operands.
- **`string.format` `%d` / `%i`** with Local-Integer args.
- **Subtype across function call boundaries** — propagate caller's Integer arg into callee's parameter slot.
- **ForNumeric induction variable** subtype tracking.
- **UnaryOp `-x`** Integer subtype propagation.
- **Integer comparison fast path** via `cmpi` when both operands are Integer subtype.

## References

- [ADR 0211](0211-math-tointeger-static-shape.md) — static literal foundation; this ADR extends to Locals.
- [ADR 0232](0232-local-number-subtype.md) — `LocalInfo::subtype` post-pass.
- [ADR 0234](0234-local-subtype-propagation.md) — BinOp propagation that benefits this arm.
- [Lua 5.4 §6.7 `math.tointeger`](https://www.lua.org/manual/5.4/manual.html#pdf-math.tointeger).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M8 milestone close.
