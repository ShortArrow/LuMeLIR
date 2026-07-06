# 0303. i64 slots for Integer locals — design (F2-R1)

- **Status:** Design — implementation split into R1-a/b/c
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Problem (from ADR 0300 probes)

Number locals live in f64 slots. Two consequences violate Lua §3.4.1:

1. `math.maxinteger + 1 == math.mininteger` → `false` (must wrap; f64 can't even *hold* maxinteger exactly — 2^63-1 rounds to 2^63).
2. `local big = 9007199254740992; big + 1` → `9.0072e+15` (precision lost on slot round-trip; the static-fold path is exact but the runtime slot path isn't).

## Decision: subtype-keyed i64 slots

Number-kind locals whose `NumberSubtype` is `Integer` get **i64 slots**; `Float` / `Unknown` keep f64. No new `ValueKind` — the static key is the existing `LocalInfo::subtype` (ADR 0232), which codegen already receives at every site via `locals: &[LocalInfo]`.

Why not `ValueKind::Integer` (option (a) in ADR 0300)? The kind lattice is consumed by ~57 `ValueKind::Function(arity)`-style matches plus the param-kind inference, dispatch signatures, and widening rules — a change there ripples into HIR semantics. Subtype-keyed slots confine the change to codegen's slot mechanics; HIR is already done (classification exists since ADR 0234, extraction since ADR 0301).

Why not NaN-boxing? i64 doesn't fit in a NaN payload (52 bits).

## Invariants

- **I1**: a slot's MLIR type is fixed at alloca time from `(kind, subtype)`; `subtype` is final after `propagate_number_subtype` (it already runs before codegen).
- **I2**: every read/write of a Number slot must consult `locals[idx].subtype` and use the matching load/store type. Mixed flows convert explicitly: Integer→Float `sitofp`, Float→Integer never implicit (HIR classification prevents it — an Integer-subtyped slot only ever receives Integer-classified RHS by the `merge` rule).
- **I3**: values crossing into dynamic contexts (TaggedValue payloads, table elements, varargs pack, call args to f64-typed params) convert `sitofp` at the boundary — precision loss beyond 2^53 at those boundaries is a *documented residual* until a `TAG_INTEGER` lands (out of scope here).

## Site enumeration (132 slot consumers audited by grep; grouped)

| Group | Sites (approx) | R1 step |
|---|---|---|
| Slot alloc (`emit_alloca_slot_for_kind` + param allocas) | 3 | a |
| `HirExprKind::Local` read | 1 chokepoint | a |
| `LocalInit` / `Assign` store | 2 chokepoints | a |
| `print` / `tostring` of Integer-subtyped Local | 2 (ADR 0214 fast path extends) | a |
| BinOp arith on two Integer operands (`addi subi muli`, floordiv `divsi`+floor-fix, mod `remsi`+sign-fix, wraparound = LLVM's natural i64 overflow) | 1 chokepoint (emit_binop) | b |
| BinOp comparisons (`cmpi` vs `cmpf`, incl. `==` int/float cross per §3.4.4) | 1 chokepoint | b |
| Bitwise ops (drop the current f64→i64→f64 round-trip for Integer slots) | 1 | b |
| Unary neg | 1 | b |
| Boundary conversions: call args, params, ret slots, TaggedValue stores, table writes, vararg pack, libm/libc args | ~15 | c |
| `math.maxinteger` / `mininteger` constants → Integer-subtyped | 2 | a |

## Step decomposition (each = one session goal)

- **F2-R1-a** — slot representation + Local read/write/print + integer constants. Gate: activation only when a local's *entire* def-use set stays inside group-a sites (checked by a conservative HIR pre-pass `integer_slot_eligible`); everything else keeps f64. Probe target: `local big = 9007199254740992; local b2 = big; print(b2)` exact.
- **F2-R1-b** — BinOp/UnaryOp on i64 + eligibility widened to arithmetic. Probe targets: both ADR 0300 gaps (`maxinteger + 1` wraps; `big + 1` exact).
- **F2-R1-c** — boundary conversions + eligibility widened to everything (drop the pre-pass gate, subtype alone decides). Residual: dynamic-context precision beyond 2^53 documented pending `TAG_INTEGER`.

The `integer_slot_eligible` gate is the staging trick that keeps each step shippable with all tests green — same discipline as F1-C's stub → pack → spread arc.

## Sessions

R1-a: 1. R1-b: 1. R1-c: 1-2. Total 3-4 (ADR 0300 said 1-2 — revised upward after the 132-site audit; still well under the original F2 6-10).

## References

- ADR 0232 — NumberSubtype.
- ADR 0300 — F2 audit (gap R1).
- ADR 0301 — classifier extraction.
- ADR 0296/0297/0298 — the step-gated arc pattern this follows.
- Lua 5.4 §3.4.1 (wraparound), §3.4.4 (int/float comparison).
