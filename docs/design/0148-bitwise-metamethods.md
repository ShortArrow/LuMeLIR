# 0148. Bitwise Metamethods (`__band` / `__bor` / `__bxor` / `__bnot` / `__shl` / `__shr`) for Table

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Sibling of [ADR 0147](0147-arith-metamethods.md). Lua spec §3.4.2 documents 6 bitwise metamethods:

- 5 binary: `__band` (`&`), `__bor` (`|`), `__bxor` (`~`), `__shl` (`<<`), `__shr` (`>>`).
- 1 unary: `__bnot` (unary `~`).

The reason for splitting from 0147: bitwise ops have the [ADR 0114](0114-phase2-emit-f2i-gate-sweep.md) integer gate (`emit_check_integer_arg`) on Number operands — NaN / Inf / non-integer trap. Table operands skip the gate (the metamethod handles everything). Bundling the two ADRs would conflate the integer-gate concern with the metamethod-dispatch concern.

## Scope (literal)

**6 bitwise metamethods, Table operand(s)**:
- Binary: `(Table, Table) → Number`. Lhs metatable probe.
- Unary `__bnot`: `(Table) → Number`.

Out of scope:

- ❌ Mixed Table/Number bitwise operands.
- ❌ Rhs-fallback.
- ❌ Non-Function metafield, non-Number return.
- ❌ TaggedValue runtime Table-tag dispatch.
- ❌ Integer-gate interaction (the gate fires on Number operands of bitwise; Table operands skip it entirely — the metamethod owns the result).

## Decision

### HIR

`lower_expr` BinOp arm: relax the bitwise kind check (currently Number-only) to accept `(Table, Table)` for the 5 binary bitwise ops.

`lower_expr` UnaryOp `BitNot` arm: accept `Table`.

### Codegen

Extend `arith_metamethod_field_name` (ADR 0147) to cover the 5 binary bitwise ops:

| BinOp | Field global |
|---|---|
| `BitAnd` | `s_metatable_band_field_name` |
| `BitOr` | `s_metatable_bor_field_name` |
| `BitXor` | `s_metatable_bxor_field_name` |
| `Shl` | `s_metatable_shl_field_name` |
| `Shr` | `s_metatable_shr_field_name` |

`emit_expr` UnaryOp arm: when `op == BitNot && operand_kind == Table`, route to `emit_unary_arith_via_metamethod` with `s_metatable_bnot_field_name`.

The dispatch helpers from ADR 0147 (`emit_arith_via_metamethod` / `emit_unary_arith_via_metamethod`) are reused verbatim — they're parameterised by field-name global, and `sig = (Table, Table) → Number` / `(Table) → Number` applies identically.

### Metamethod-aware kind refinement

| Key | Forced signature |
|---|---|
| `__band` / `__bor` / `__bxor` / `__shl` / `__shr` | `(Table, Table) → Number` |
| `__bnot` | `(Table) → Number` |

## Alternatives considered

- **Bundle bitwise into ADR 0147 arith.** Rejected per Codex per-decision precedent; the integer gate is a separate concern.
- **Apply the integer gate to `__band` etc.'s **return** value**. Rejected — the metamethod's return is whatever the user wrote; if downstream code feeds it into another bitwise op, that downstream call's gate fires naturally on the Number.
- **Per-op ADRs**. Rejected — same shape as ADR 0147, per-family bundle keeps reviewable.

## Consequences

**Positive**
- Bitwise idioms (bitfields, packed flags, custom hash dispatch) work for Table-Table.
- Codegen reuses ADR 0147 helpers verbatim — net change is 6 globals + 6 enum-mapper rows.

**Negative**
- 6 more module globals (the field-name strings).
- Codegen `arith_metamethod_field_name` mapper grows.

**Locked in until superseded**
- Table-Table (or Table-unary) only.
- Lhs-side metatable probe.
- Function-form, Number return.

## Documentation updates

- [x] §4 LIC — new `LIC-bitwise-metamethods-1`.
- [x] §7 — closes bitwise Table-Table item.
- [x] §8 — adds 0148.

## Test count delta

```
Step 0:   1342 (after ADR 0147)
C2 (6 e2e Red Day 0):  1342 → 1342
C3 (impl): 1342 → 1348
```

## Critical files

- `src/hir/mod.rs`:
  - BinOp bitwise arm: widen to accept (Table, Table).
  - UnaryOp BitNot arm: accept Table.
  - Metamethod-aware refinement walk: 6 new keys.
- `src/codegen/emit.rs`:
  - 7 new module globals (6 field names + reuse of `s_arith_no_metamethod`).
  - `arith_metamethod_field_name` extends to 5 binary bitwise ops.
  - `emit_expr` UnaryOp arm gets Table-BitNot route.
- `tests/phase2_6plus_bitwise_metamethods.rs` (NEW) — 6 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Integer gate fires for Table operand on bitwise | The Table-Table arm intercepts BEFORE `emit_binop` / integer gate. Tests pin. |
| BitNot Table operand not routed | Codegen `UnaryOp::BitNot` adds explicit Table-arm check. |
| Empty candidate set crashes | Skip dispatch; trap directly. |

## Future work

- Mixed-operand bitwise.
- Rhs-fallback.
- Non-Function, non-Number return.
- TaggedValue runtime Table-tag dispatch.

## References

- [ADR 0022](0022-phase2-2c-floor-and-bitwise.md) — original bitwise ops.
- [ADR 0114](0114-phase2-emit-f2i-gate-sweep.md) — integer gate.
- [ADR 0147](0147-arith-metamethods.md) — arith sibling; helper reuse.
- Lua 5.4 reference manual §3.4.2 — bitwise + metamethods.
