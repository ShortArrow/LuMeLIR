# 0143. `__concat` Metamethod for `..` BinOp

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Tier 2 metamethod ADR #2, sibling to [ADR 0142](0142-tostring-metamethod.md). Lua spec §3.4.6 / §6.1: when `..` (concat) is applied to operands that are not both string-coercible, the runtime consults `__concat` on the first or second operand's metatable, calls it with both operands, and returns its result.

Today `coerce_to_string` (`src/hir/mod.rs:414`) rejects Table operands at HIR time. With ADR 0141 (anon-fn param refinement) and ADR 0142's `emit_dispatch_chain_from_slot_ptr` helper landed, the prerequisites are met.

## Scope (literal)

**`Table .. Table` only**, Function-form `__concat`, returns String. Out of scope:

- ❌ `Table .. String` / `String .. Table` mixed-operand. Future ADR.
- ❌ Right-side-only `__concat` (when lhs has no metatable / no `__concat` but rhs does). Lua spec checks both; Phase 1 checks lhs only.
- ❌ Non-Function `__concat` (Lua spec is Function-only here, matching).
- ❌ Non-String return value (we statically constrain to String via candidate filter).

## Decision

### HIR

`coerce_to_string` (`src/hir/mod.rs:414`): the Table arm returns the expression as-is (no auto-wrap with `tostring`). The Function arm continues to reject.

`Builtin::ToString`'s permissive arg-kind check already covers Table (ADR 0142).

`BinOp::Concat`'s overall return kind stays `String` — the codegen-side dispatch always yields a String ptr.

### Codegen

`src/codegen/emit.rs::emit_expr` BinOp arm: when `op == BinOp::Concat` and **both** `lhs_kind == Table && rhs_kind == Table`, route to a new helper `emit_concat_via_metamethod` instead of the existing `emit_concat` (which expects String ptrs).

`emit_concat_via_metamethod(lhs_t, rhs_t, functions, ...)`:

1. Compile-time candidate filter: all user fns with sig `(Table, Table) → String`.
2. Empty → fall back to a runtime trap (a string-concat on Table without metamethod is a TypeError per Lua spec).
3. Load `mt_ptr = *(lhs_t + TABLE_OFF_METATABLE)`.
4. If null → trap (`s_concat_no_metamethod` — new global).
5. Else probe `mt["__concat"]` via `emit_hash_lookup_into_tagged_slot(NilOnMissing)`.
6. If tag != TAG_FUNCTION → trap.
7. Else dispatch via `emit_dispatch_chain_from_slot_ptr(probe_slot, sig=(Table,Table)→String, candidates, [lhs_t, rhs_t])`.
8. Result is the String ptr.

### Pass-1.5 metamethod-aware refinement (HIR)

Extend ADR 0142's metamethod-aware refinement walk in `lower()`: for top-level `IndexAssign(target, Str("__concat"), FunctionExpr)`, force the resolved FuncId's `params` to `[Table, Table]` and ret to `[String]`.

The Pass-1.5 walker would otherwise default to `[Number; arity]` (the `tostring(t)`-style call site invisibility issue).

## Alternatives considered

- **Bundle mixed-operand support (`Table .. String` etc.) in this ADR**. Rejected — the mixed case needs to know which side has the metamethod, and the candidate filter becomes per-arg shape conditional. Defer for ADR-per-decision.
- **Check both sides' metatables in lhs-then-rhs order**. Rejected for Phase 1 — adds a branch on every concat without observable benefit until users actually write the rhs-only case.
- **Fall back to `tostring(t) .. tostring(t)`** instead of trapping when `__concat` is absent. Rejected — Lua spec is explicit that this is a TypeError.
- **String-form `__concat` (where the metafield is a String, not a Function)**. Rejected — Lua spec actually requires Function-form here, unlike `__tostring`.

## Consequences

**Positive**
- The `__concat` idiom works for Table .. Table: `setmetatable(t, mt); mt.__concat = function(a, b) return "..." end; print(t .. t)`.
- Reuses ADR 0142's `emit_dispatch_chain_from_slot_ptr` helper — no codegen surface growth beyond the new metamethod arm.

**Negative**
- Trap-on-no-metamethod for Table operand is a behaviour shift from "HIR rejects" to "runtime traps". Acceptable — both deny the operation, just at different layers.
- Mixed-operand cases remain explicit reject (still HIR-time today, until a future ADR widens).

**Locked in until superseded**
- Table-Table only.
- Lhs-side metatable probe only.
- Function-form only.

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC — new `LIC-concat-metamethod-1`.
- [x] §7 open questions — closes `__concat` Function-form Table-Table item; opens mixed-operand / rhs-fallback as new follow-up.
- [x] §8 ADR index — adds 0143.

## Test count delta

```
Step 0:   1320 (after ADR 0142)
C2 (4 e2e Red Day 0):  1320 → 1320
C3 (impl): 1320 → 1324
```

## Critical files

- `src/hir/mod.rs`:
  - `coerce_to_string` Table arm returns input as-is.
  - Metamethod-aware refinement walk adds `__concat` arm with `(Table, Table) → String`.
- `src/codegen/emit.rs`:
  - 1 new global `s_concat_no_metamethod`.
  - `emit_expr` Concat arm routes Table-Table through `emit_concat_via_metamethod`.
  - `emit_concat_via_metamethod` helper (~120 LOC).
- `tests/phase2_6plus_concat_metamethod.rs` (NEW) — 4 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Empty candidate set crashes the dispatch | Skip dispatch; emit trap directly. Test 4 pins. |
| Existing String-String concat regresses | Codegen arm routes through `emit_concat_via_metamethod` only when BOTH lhs and rhs are Table. String paths untouched. Existing concat tests are the regression net. |
| TaggedValue operand with runtime Table tag bypasses the arm | Out of scope; static-Table only. TaggedValue path remains rejected (existing `coerce_to_string` arm). |
| Recursion through `__concat` calling itself | Same as any user-fn recursion. Stack overflow on infinite. |

## Future work

- ADR 0144 = E comparison metamethods (`__eq` / `__lt` / `__le`).
- Mixed-operand `__concat` (`Table .. String`, `String .. Table`).
- Rhs-fallback `__concat` (when lhs lacks the metafield).
- TaggedValue runtime Table-tag dispatch.

## References

- [ADR 0025](0025-phase2-7b-string-concat.md) — original `..` concat path.
- [ADR 0082](0082-phase2-5x-callee-dispatch.md) — IndirectDispatch chain.
- [ADR 0141](0141-anon-fn-indexassign-param-refine.md) — anon-fn param refinement (mt.__concat = function(a, b)... shape).
- [ADR 0142](0142-tostring-metamethod.md) — sibling metamethod ADR; `emit_dispatch_chain_from_slot_ptr` helper reused.
- Lua 5.4 reference manual §3.4.6 — `__concat` semantics.
