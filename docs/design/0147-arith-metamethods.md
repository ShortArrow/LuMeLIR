# 0147. Arithmetic Metamethods (`__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__idiv` / `__unm`) for Table

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Tier 2 metamethod ADR #5 — the largest per-family bundle. Lua spec §3.4.1: arithmetic ops on non-Number operands consult metamethods on either operand's metatable. Eight ops:

- 7 binary: `__add` (`+`), `__sub` (`-`), `__mul` (`*`), `__div` (`/`), `__mod` (`%`), `__pow` (`^`), `__idiv` (`//`).
- 1 unary: `__unm` (unary `-`).

Today HIR rejects all non-Number operands with TypeMismatch (`src/hir/mod.rs:3867`). The 7 binary ops follow the exact same shape (`(Table, Table) → Number`); `__unm` is `(Table) → Number`. Per-family bundle (Codex precedent ADR 0089 arith dispatch) keeps the codegen helper parameterised by op + metamethod-field name.

Bitwise metamethods (`__band` / `__bor` / `__bxor` / `__bnot` / `__shl` / `__shr`) are out of scope here — same shape, but bitwise ops in the HIR have stronger integer requirements (ADR 0114 `emit_check_integer_arg` gate) that interact differently. Separate ADR.

## Scope (literal)

**8 arith metamethods, Table operand(s)**:
- Binary: lhs Table-Table → `(Table, Table) → Number`. Lhs metatable probe only.
- Unary `__unm`: `(Table) → Number`.

Out of scope:

- ❌ Mixed Table/Number operands. Deferred follow-up (would require per-op-and-side candidate filtering).
- ❌ Bitwise metamethods.
- ❌ Rhs-fallback (when lhs has no metafield but rhs does).
- ❌ Non-Function metafield.
- ❌ Non-Number return type from the metamethod.
- ❌ TaggedValue runtime Table-tag dispatch.

## Decision

### HIR

`lower_expr` BinOp arm: relax the arith kind check for the 7 binary ops to accept `(Table, Table)`. Bitwise ops still reject.

`lower_expr` UnaryOp `Neg` arm: accept `Table` operand.

Result kind: stays `Number` (the metamethod is statically constrained to return Number per the candidate filter).

### Codegen

Two new helpers in `src/codegen/emit.rs`:

- `emit_arith_via_metamethod(op_field_name: &str, lhs_t, rhs_t, functions, ...)` — load `mt_ptr = *(lhs + 32)`, probe `mt[op_field_name]`, dispatch via `emit_dispatch_chain_from_slot_ptr` (ADR 0142) with `sig = (Table, Table) → Number`. Trap on missing.
- `emit_unary_arith_via_metamethod` for `__unm` — same shape but `sig = (Table) → Number` and arg list `[t]`.

`emit_expr` BinOp arm: after Concat / String-cmp / Table-cmp paths, add Table-Table arith path that picks the metamethod-field name by op:

```rust
let field = match op {
    BinOp::Add => "s_metatable_add_field_name",
    BinOp::Sub => "s_metatable_sub_field_name",
    BinOp::Mul => "s_metatable_mul_field_name",
    BinOp::Div => "s_metatable_div_field_name",
    BinOp::Mod => "s_metatable_mod_field_name",
    BinOp::Pow => "s_metatable_pow_field_name",
    BinOp::FloorDiv => "s_metatable_idiv_field_name",
    _ => unreachable!(),
};
```

`emit_unary` UnaryOp::Neg arm: when operand is Table, route to `emit_unary_arith_via_metamethod`.

### New module globals

8 field-name globals + 1 trap message:

- `s_metatable_add_field_name` ("__add"), `s_metatable_sub_field_name`, `s_metatable_mul_field_name`, `s_metatable_div_field_name`, `s_metatable_mod_field_name`, `s_metatable_pow_field_name`, `s_metatable_idiv_field_name`, `s_metatable_unm_field_name`.
- `s_arith_no_metamethod` ("attempt to perform arithmetic on a table value").

### Metamethod-aware kind refinement

Extend the post-Pass-1.5 walk for the 8 keys:

| Key | Forced signature |
|---|---|
| `__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__idiv` | `(Table, Table) → Number` |
| `__unm` | `(Table) → Number` |

## Alternatives considered

- **Per-op ADRs.** Rejected per Codex per-family precedent (ADR 0089).
- **Bundle bitwise in this ADR.** Rejected — bitwise has the ADR 0114 integer gate; merging would conflate two concerns.
- **Allow non-Number return type.** Rejected — would require widening the candidate filter and HIR result-kind inference. Deferred.
- **Rhs-fallback.** Rejected for first cut, same as ADRs 0143 / 0144.

## Consequences

**Positive**
- All 7 binary arith ops + unary `-` work between Tables when the metamethod is present. Closes a major Lua compatibility gap (vectors / matrices / complex numbers / fixed-point).
- Codegen helper reuse: `emit_arith_via_metamethod` is parameterised by field name, so the 7 binary arms share ~100 LOC of structure.

**Negative**
- Adds 8 new module globals (field name strings).
- Codegen `emit_expr` BinOp arm grows ~40 LOC for the per-op field-name dispatch.

**Locked in until superseded**
- Table-Table only (Table-Number / Number-Table deferred).
- Lhs-side metatable probe only.
- Function-form only.
- Number return type only.

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC — new `LIC-arith-metamethods-1`.
- [x] §7 open questions — closes arith Table-Table item; opens mixed-operand / bitwise / rhs-fallback / non-Number return as follow-ups.
- [x] §8 ADR index — adds 0147.

## Test count delta

```
Step 0:   1334 (after ADR 0146)
C2 (8 e2e Red Day 0):  1334 → 1334
C3 (impl): 1334 → 1342
```

## Critical files

- `src/hir/mod.rs`:
  - BinOp arith arm: widen kind check to accept (Table, Table) for the 7 non-bitwise ops.
  - UnaryOp Neg arm: accept Table operand.
  - Metamethod-aware refinement walk extended for the 8 keys.
- `src/codegen/emit.rs`:
  - 9 new module globals (8 field names + 1 trap message).
  - `emit_expr` BinOp arith arm routes Table-Table through new helper.
  - `emit_unary` UnaryOp Neg arm routes Table through new helper.
  - `emit_arith_via_metamethod` + `emit_unary_arith_via_metamethod` (~300 LOC total).
- `tests/phase2_6plus_arith_metamethods.rs` (NEW) — 8 e2e (1 per metamethod).
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Empty candidate set → unconditional trap | Intended; tests pin the trap path for missing metafields. |
| Per-op dispatch wires the wrong field name | Single `match op { ... => "..." }` table; test count covers each op individually. |
| `__unm` operand confusion (unary vs binary signatures) | Dedicated unary helper. Separate signature. Test pins. |
| `Table - Number` accidentally matches the new path | HIR check requires BOTH sides Table; mixed shapes hit the existing TypeMismatch reject. |

## Future work

- Mixed-operand (`Table + Number`, etc.).
- Bitwise metamethods (`__band` etc., separate ADR).
- Rhs-fallback when lhs has no metafield.
- Non-Function metafields, non-Number return types.
- TaggedValue runtime Table-tag dispatch.

## References

- [ADR 0009](0009-phase2-2a-arith-operators.md) — original arith.
- [ADR 0022](0022-phase2-2c-floor-and-bitwise.md) — FloorDiv + bitwise.
- [ADR 0089](0089-phase2-7p-tagged-arith-coerce.md) — TaggedValue arith dispatch (per-family bundle precedent).
- [ADR 0114](0114-phase2-emit-f2i-gate-sweep.md) — integer gate (interacts with bitwise; bundled deferral).
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` helper.
- [ADR 0143](0143-concat-metamethod.md) / [ADR 0144](0144-comparison-metamethods.md) / [ADR 0146](0146-call-metamethod.md) — sibling Table-operand metamethod ADRs.
- Lua 5.4 reference manual §3.4.1 — arithmetic + `__add` etc. semantics.
