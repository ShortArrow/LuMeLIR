# 0144. Comparison Metamethods (`__eq` / `__lt` / `__le`) for Table-Table

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Tier 2 metamethod ADR #3, sibling to [ADR 0142](0142-tostring-metamethod.md) (`__tostring`) and [ADR 0143](0143-concat-metamethod.md) (`__concat`). Lua spec §3.4.4 documents `__eq` / `__lt` / `__le`:

- `a == b`: raw-equal short-circuits to `true`. Otherwise, if both operands are tables, look up `__eq` on the metatable. Absent → `false`.
- `a < b`: tables require `__lt`; absent → TypeError.
- `a <= b`: tables require `__le`; absent → TypeError (Lua 5.4 removed the fallback to `not (b < a)`).
- `a > b` / `a >= b`: lowered as the swapped form of `<` / `<=`.

Today HIR rejects Table-Table for `<` / `<=` / `>` / `>=` with TypeMismatch (`src/hir/mod.rs:operand kind check`); codegen for `a == b` between two tables emits an `arith.cmpf` on `!llvm.ptr` and trips verification. ADRs 0141 (anon-fn refinement) and 0142's `emit_dispatch_chain_from_slot_ptr` helper supply the prerequisites.

## Scope (literal)

**Table-Table only** for all three comparison metamethods. Function-form only. Bundled per Codex per-family ADR precedent (ADR 0089 arith dispatch). Out of scope:

- ❌ Mixed Table/non-Table operands (Lua spec rejects these for `<` / `<=` anyway; `==` of different types is statically `false`).
- ❌ Rhs-fallback metamethod (`__eq` / `__lt` / `__le` on the rhs's metatable when the lhs lacks it). Lua spec checks both; Phase 1 lhs only for `__lt` / `__le`; `__eq` checks either side.
  - Correction: for `__eq` Phase 1 still only checks lhs's metatable to keep scope tight.
- ❌ Non-Function metafield values.
- ❌ `__eq` with operands of different types (Lua spec: `nil == 0` is `false`, never consults `__eq`).
- ❌ TaggedValue runtime Table-tag dispatch.

## Decision

### HIR

`lower_expr` BinOp arm: relax the kind-compatibility check for `Eq / Ne / Lt / Le / Gt / Ge` to accept `(Table, Table)` (in addition to existing accepted shapes).

The result kind of all six ops stays `Bool` — codegen-side dispatch always yields an `i1`.

### Codegen

`src/codegen/emit.rs::emit_expr` BinOp arm, after the existing Concat / String-cmp paths, add:

```rust
if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    && infer_kind(lhs, locals, functions) == ValueKind::Table
    && infer_kind(rhs, locals, functions) == ValueKind::Table
{
    return Ok(emit_table_cmp_via_metamethod(
        context, block, *op, lhs_val, rhs_val, functions, types, loc,
    ));
}
```

`emit_table_cmp_via_metamethod(op, lhs_t, rhs_t)`:

- For `Eq`: ptr-equality short-circuit → `true`. Else dispatch on `lhs.metatable.__eq` with `(lhs, rhs)`; absent → `false`.
- For `Ne`: complement of `Eq`.
- For `Lt`: dispatch `lhs.metatable.__lt` with `(lhs, rhs)`; absent → trap (`s_cmp_no_metamethod`, new global).
- For `Le`: dispatch `lhs.metatable.__le` with `(lhs, rhs)`; absent → trap.
- For `Gt`: swap → `Lt` with `(rhs, lhs)`.
- For `Ge`: swap → `Le` with `(rhs, lhs)`.

The dispatch uses ADR 0142's `emit_dispatch_chain_from_slot_ptr` with `sig = (Table, Table) → Bool`. Candidate filter: user fns with that signature.

### HIR metamethod-aware refinement

Extend the metamethod-aware refinement walk in `lower()` for the three new keys:

| Key | Forced signature |
|---|---|
| `__eq` | `(Table, Table) → Bool` |
| `__lt` | `(Table, Table) → Bool` |
| `__le` | `(Table, Table) → Bool` |

`params[0..2] = [Table, Table]` and `ret_kinds = [Bool]` post-Pass-1.5.

## Alternatives considered

- **Per-op ADRs (0144 / 0145 / 0146)**. Rejected — the three are structurally identical (`(Table, Table) → Bool`); per Codex per-family bundling precedent (ADR 0089 arith dispatch), one ADR is reviewer-friendlier.
- **Restore Lua 5.3 `__le` fallback to `not (b < a)`**. Rejected — Lua 5.4 removed it; we match 5.4.
- **`__eq` on differing types**. Rejected — Lua spec returns `false` immediately without consulting `__eq`. Matches.
- **Implicit rhs-fallback for `__lt` / `__le`**. Rejected for scope; deferred to follow-up.

## Consequences

**Positive**
- `a == b`, `a < b`, `a <= b`, `a > b`, `a >= b` all work between Tables when the metamethod is present.
- Reuses `emit_dispatch_chain_from_slot_ptr` (ADR 0142) — no new dispatch surface.
- ret_kinds refinement is per-key principled — future per-metamethod ADRs follow the same pattern.

**Negative**
- Codegen helper grows ~150 LOC (three closely-related arms in one function).
- Static `(Table) == (Number)` etc. still relies on existing typed-equality short-circuits (Number-Number etc.); only Table-Table goes through the metamethod path.

**Locked in until superseded**
- Table-Table only.
- Lhs-side metatable probe only.
- Function-form only.

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC — new `LIC-comparison-metamethods-1`.
- [x] §7 open questions — closes `__eq` / `__lt` / `__le` Table-Table item; opens rhs-fallback / mixed-operand as new follow-up.
- [x] §8 ADR index — adds 0144.

## Test count delta

```
Step 0:   1324 (after ADR 0143)
C2 (6 e2e Red Day 0):  1324 → 1324
C3 (impl): 1324 → 1330
```

## Critical files

- `src/hir/mod.rs`:
  - BinOp comparison kind-check widens to accept Table-Table.
  - Metamethod-aware refinement walk adds `__eq` / `__lt` / `__le` arms.
- `src/codegen/emit.rs`:
  - 1 new global `s_cmp_no_metamethod`.
  - `emit_expr` BinOp arm routes Table-Table comparisons through new helper.
  - `emit_table_cmp_via_metamethod` (~200 LOC for the three op families + Eq ptr-eq short-circuit).
- `tests/phase2_6plus_comparison_metamethods.rs` (NEW) — 6 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Empty candidate set + `__lt` / `__le` requested → always trap | Intended (Lua spec). Tests pin. |
| `Eq` ptr-equality short-circuit returns wrong answer for aliased Locals | `local a = {}; local b = a; print(a == b)` — both slot ptrs point to the same Table ptr, `ptrtoint+cmpi` returns true. Test pins. |
| `Gt` / `Ge` swap accidentally calls `__gt` / `__ge` (non-existent) | The codegen helper translates Gt→Lt(swap) and Ge→Le(swap) BEFORE dispatch. Test pins. |
| `__eq` returning a non-Bool value | Compile-time candidate filter `(Table, Table) → Bool` excludes non-Bool returns. |
| Existing Number-Number / String-String comparisons regress | Codegen routes through metamethod only when BOTH operands are static Table. All other shapes hit existing paths. |

## Future work

- Rhs-fallback metamethod (when lhs lacks `__eq` / `__lt` / `__le`).
- Mixed Table/non-Table operands (where Lua spec permits, mostly `__eq`).
- TaggedValue runtime Table-tag dispatch.
- ADR 0145 = N GC strategy.
- ADR 0146+ = `__call` / `__index = Function` / etc.

## References

- [ADR 0010](0010-phase2-2b-comparisons.md) — original Lt/Le/Gt/Ge.
- [ADR 0066](0066-phase2-6c-tag-hetero-eq.md) — runtime tag-dispatch eq.
- [ADR 0082](0082-phase2-5x-callee-dispatch.md) — IndirectDispatch chain.
- [ADR 0141](0141-anon-fn-indexassign-param-refine.md) — anon-fn refinement.
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` helper (reused).
- [ADR 0143](0143-concat-metamethod.md) — sibling metamethod ADR.
- Lua 5.4 reference manual §3.4.4 — relational ops + `__eq` / `__lt` / `__le` semantics.
