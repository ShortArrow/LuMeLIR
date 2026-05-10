# 0089. Phase 2.7p-tagged-arith-coerce: TaggedValue Arith Operand Coercion Chokepoint

- **Status:** Accepted
- **Date:** 2026-05-10
- **Deciders:** ShortArrow

## Context

ADR 0088 (`d460fea`, 2026-05-10) closed `LIC-2.6b-hash-missing-key-read-1`,
leaving **only `LIC-2.7p-arith-coerce-tagged-1`** as the lone pending
LIC at **28 / 28 / 1**. ADR 0077 introduced String → Number coercion
for **static String** arith operands via `HirExprKind::ArithStringCoerce`
+ `emit_tonumber_for_arith`, but TaggedValue operands
(`local x = t["k"]; x + 1` where x is runtime String) trap because:

- HIR cannot statically resolve x's kind (it's TaggedValue, not String).
- `emit_expr` for `Local(TaggedValue)` calls
  `emit_value_slot_check_number` (`tagged.rs:389-428`) which traps
  with `s_table_type_mismatch` on any non-Number tag.
- `coerce_arith_operand_if_string` in `hir/mod.rs` only wraps operands
  whose static kind is `ValueKind::String`; TaggedValue operands skip
  the wrapping entirely.

Codex review (post-0088, 6 視点) flagged candidate A: **Strong Go 6/6**
with 3 critical guardrails:
1. **non-ad-hoc**: must NOT be "12-ops × inline tag dispatch".
2. **Trap message contract**: must define `coerce failure` (parse-fail
   on String) vs `non-numeric tag` (Bool/Nil/Function/Table operand).
3. **Op classes**: arith / bitwise / ordering have **different
   consumer contracts**; ordering must NOT silently coerce.

## Reframing

> **Index hash arith operand coercion is a runtime tag-dispatch
> chokepoint**, mirroring `emit_tagged_eq_runtime_dispatch` (the
> existing Eq/Ne pattern) and the ADR 0087 / 0088 pure-policy +
> effectful-executor split.

The dispatcher does not patch each of 12+ ops with inline tag checks.
It detects `Local(TaggedValue)` operand at the BinOp / UnaryOp lowering
and short-circuits to a single chokepoint
(`emit_load_tagged_operand_as_number`) that materialises an `f64`
according to the pure policy decision. All downstream ops
(`emit_binop`, `emit_unary`) consume the f64 unchanged.

## Decision

### `tagged.rs::policy_for_tagged_arith_operand` (pure decision)

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaggedArithOperandPlan {
    UseNumberPayload,      // tag == TAG_NUMBER
    CoerceStringToNumber,  // tag == TAG_STRING (sscanf via emit_tonumber_for_arith)
    TrapNonNumeric,        // Bool / Nil / Function / Table / Deleted
}

pub(crate) fn policy_for_tagged_arith_operand(tag: i64) -> TaggedArithOperandPlan {
    match tag {
        TAG_NUMBER => TaggedArithOperandPlan::UseNumberPayload,
        TAG_STRING => TaggedArithOperandPlan::CoerceStringToNumber,
        _          => TaggedArithOperandPlan::TrapNonNumeric,
    }
}
```

Pure tag → plan mapping. Total over the i64 tag space; the helper is
testable in isolation (no MLIR `Context`). Mirrors ADR 0087's
`policy_for_tag` shape. Future tag kinds extend coverage here without
touching the chokepoint.

### `emit.rs::s_arith_on_non_numeric` (trap message global)

`"attempt to perform arithmetic on a non-numeric value\0"` — fires
when TaggedValue operand has tag Bool/Nil/Function/Table/Deleted.
**Distinct from `s_arith_coerce_failed`** (ADR 0077 — `"attempt to
perform arithmetic on a string value"`, which fires on parse failure
during the String → Number coerce). Two distinct error conditions, two
distinct messages (codex critical #2).

### `emit.rs::emit_load_tagged_operand_as_number` (effectful chokepoint)

Single helper, signature
`(slot_ptr: Value) -> Value /* f64 */`. Internally:
1. Load tag at slot+0.
2. Recurse over `DISPATCH_TAGS = [TAG_NUMBER, TAG_STRING]` building
   nested scf.if; each known tag queries the policy and emits the
   per-plan f64 production via `emit_arith_operand_plan`.
3. Trailing `else` (no known tag matched) calls
   `emit_arith_operand_plan(TrapNonNumeric)` which exits with
   `s_arith_on_non_numeric` and yields a placeholder f64 (matches the
   `emit_tagged_eq_runtime_dispatch` placeholder pattern for scf.if
   region typing).

The dispatch is **driven by the policy enum**: `policy_for_tagged_arith_operand(head_tag)`
returns the variant, and `emit_arith_operand_plan(plan)` matches on
the variant to pick the per-plan emission. Future tag additions only
require extending `DISPATCH_TAGS` and the policy table; the per-plan
helper is the join point for the new code path.

### `emit.rs::emit_tagged_arith_runtime_dispatch` (BinOp dispatcher route)

```rust
fn emit_tagged_arith_runtime_dispatch(
    ..., op: BinOp, lhs: &HirExpr, rhs: &HirExpr, ...,
) -> Result<Option<Value<'c, 'a>>, CodegenError>
```

Returns `Some(f64)` when:
- `op ∈ {Add, Sub, Mul, Div, Mod, Pow, FloorDiv, BitAnd, BitOr, BitXor, Shl, Shr}` **AND**
- `lhs` or `rhs` is `Local(TaggedValue)`.

For each TaggedValue operand: routes through
`emit_load_tagged_operand_as_number`. For typed Number operands or
ADR 0077 `ArithStringCoerce` wrappers: passes through `emit_expr`. The
two f64s feed `emit_binop` (existing arith + bitwise lowering).

Wired into the BinOp dispatcher at `emit.rs:6866` immediately after
the Eq/Ne dispatch:
```rust
if let Some(v) = emit_tagged_arith_runtime_dispatch(...)? {
    return Ok(v);
}
```

### `emit.rs` UnaryOp dispatcher route

For `op ∈ {Neg, BitNot}` with `Local(TaggedValue)` operand: same
chokepoint via `emit_load_tagged_operand_as_number`. Inline guard
just before `emit_unary`; falls through to `emit_expr` otherwise.
Not extracted into a separate helper since it's a 4-line one-shot
(rule-of-three not met).

### Op-class scope (codex critical #3)

| Op class            | Ops                                       | Behaviour for TaggedValue operand                       |
|---------------------|-------------------------------------------|---------------------------------------------------------|
| Arith               | Add, Sub, Mul, Div, Mod, Pow, FloorDiv    | Coerce per policy; fall through to `emit_binop` arith   |
| Bitwise             | BitAnd, BitOr, BitXor, Shl, Shr           | Coerce per policy; fall through to `emit_binop` bitwise (f64 → i64 → op → f64) |
| Unary               | Neg, BitNot                               | Coerce per policy; fall through to `emit_unary`         |
| **Eq / Ne**         | Eq, Ne                                    | **Out of scope** — already handled by `emit_tagged_eq_runtime_dispatch` (ADR 0066) |
| **Ordering**        | Lt, Le, Gt, Ge                            | **Out of scope** — Lua §3.4.4: mixed-kind ordering is an error, not coercion. Existing trap behavior is correct |
| **Concat**          | `..`                                      | **Out of scope** — already auto-coerces via `tostring` (ADR 0026) |

Total in-scope ops: **14** (7 arith + 5 bitwise + 2 unary).

### Diagnostic surface

| Condition                                              | Trap message global       | Trap message text                                      |
|--------------------------------------------------------|---------------------------|--------------------------------------------------------|
| TaggedValue Bool/Nil/Function/Table/Deleted in arith   | `s_arith_on_non_numeric`  | "attempt to perform arithmetic on a non-numeric value" |
| TaggedValue String, sscanf parse failure ("abc" + 1)   | `s_arith_coerce_failed`   | "attempt to perform arithmetic on a string value" (ADR 0077, reused) |
| Static String operand parse failure (ADR 0077 path)    | `s_arith_coerce_failed`   | (unchanged, ADR 0077)                                  |

## Alternatives Considered

- **Inline 12 ops × tag dispatch inside `emit_binop`**. Codex critical
  #1 ("12 ops × inline patches"). Rejected — would replicate the
  bool-flag abstraction smell that ADR 0088 §non-ad-hoc explicitly
  retired.
- **Promote ordering ops into scope** (route `Lt/Le/Gt/Ge` through
  the same chokepoint). Lua §3.4.4 says mixed-kind ordering is an
  **error** (not coercion); silently coercing would be a spec
  violation. Out of scope.
- **Widen the dispatch chain to detect operand kinds at HIR**
  (extend `coerce_arith_operand_if_string` to TaggedValue too).
  Rejected — HIR cannot resolve TaggedValue's runtime tag, and
  introducing a runtime-dispatch HIR variant would cross the
  HIR/codegen boundary unnecessarily. The codegen-layer fix is
  cleaner.
- **Single helper `emit_load_tagged_operand_as_number` without the
  recursive dispatch chain**. The flat scf.if version (TAG_NUMBER
  → else if TAG_STRING → else trap) was the v1 prototype, but the
  pure-policy enum was not actually consulted — only the
  *implementation* matched the policy. Refactored to an
  `emit_tagged_arith_dispatch_chain` recursion that **calls**
  `policy_for_tagged_arith_operand` per known tag, ensuring future
  policy changes propagate. Same MLIR shape, stronger structural
  link between `tagged.rs` and `emit.rs`.

## Consequences

- **`LIC-2.7p-arith-coerce-tagged-1` → resolved.**
- **LIC totals: 28 / 28 / 1 → 28 / 28 / 0.** Phase 2 tagged-semantics
  reaches **"consumer coverage complete"** milestone — every
  TaggedValue consumer (print / type / tostring / eq / arith / hash
  read / hash write / iter) now has a runtime tag-dispatch chokepoint.
- **Test totals: 999 → 1013** (3 unit + 9 e2e + 1 existing test
  flip + 0 net via 2 regression pins that were green Day 0).
- **Source LOC delta**:
  - `tagged.rs`: +50 LOC (enum + policy + 3 unit tests)
  - `emit.rs`: +250 LOC (global + chokepoint + dispatch chain +
    plan emitter + BinOp dispatcher + UnaryOp wire + extracted
    `tagged_local_idx` + `is_tagged_arith_eligible_op`)
  - **net ~+300 LOC** (no retirements in this ADR)
- **User-visible behaviour shift**:
  - `local t = {"5"}; print(t[1] + 1)` → was trap; now `6`
  - `local t = {true}; print(t[1] + 1)` → was trap with
    `s_table_type_mismatch`; now trap with `s_arith_on_non_numeric`
  - Static String coerce (`"5" + 1 → 6`) **unchanged** (ADR 0077
    path)
  - Existing test `tests/phase2_6c_tag_hetero.rs::arith_on_tagged_local_traps_for_string`
    renamed to `arith_on_tagged_local_coerces_parseable_string` and
    flipped from trap-pin to success-pin.

### Carry-overs

- **Ordering on TaggedValue** (`Lt/Le/Gt/Ge`) — out of scope per Lua
  §3.4.4 (mixed-kind ordering is an error). If same-kind String-vs-
  String ordering needs runtime support for TaggedValue locals
  later, a separate ADR.
- **Bitwise integer-form check** (Lua §3.4.2: bitwise on float
  requires exact integer value). Current f64 → i64 via `emit_f2i`
  rounds; ADR 0089 does not introduce a stricter check. Future
  Tidy First if Lua-spec exact-integer compliance matters.
- **`tmp slot → check_number → load` micro-helper** (codex
  post-0088 candidate C) — independent refactor; ADR 0089 does not
  touch the surface.

## TDD Process

1. **Red.** 9 e2e in new `tests/phase2_6c_tag_hetero.rs`-adjacent
   `tests/phase2_7p_tagged_arith_coerce.rs`:
   - 4 success cases (Add / Mul / BitAnd / Unary Neg on TaggedValue
     String); behaviour-change pins asserting NEW result.
   - 4 trap pins (Bool / Nil / Function / Table) asserting NEW
     `s_arith_on_non_numeric`.
   - 1 parse-fail pin (TaggedValue String "abc") asserting NEW
     `s_arith_coerce_failed`.
   Plus 2 regression pins (Day 0 green throughout) and 1 existing
   test flip in `tests/phase2_6c_tag_hetero.rs`.
2. **Green.**
   - Step 1: pure decision module (3 unit tests Green).
   - Step 2: trap global + chokepoint helper (build green only).
   - Step 3: BinOp dispatcher + wire-up → 8 of 9 e2e flip + 1
     existing flip (1003 → 1012).
   - Step 4: UnaryOp wire-up → last e2e (`unary_neg`) flips
     (1012 → 1013).
3. **Refactor.** Extracted `tagged_local_idx` from
   `emit_tagged_eq_runtime_dispatch` to module scope so the new
   arith dispatcher reuses it (rule-of-three: Eq + arith + future
   ordering candidates). Recursive `emit_tagged_arith_dispatch_chain`
   replaced the v1 flat scf.if to drive emission from the policy
   enum (codex non-ad-hoc preference).

## Documentation updates

- [x] `docs/design/tagged-semantics.md` §3 consumer matrix —
      Arith / Bitwise / Unary rows added (TaggedValue operand →
      coerce-or-trap behaviour).
- [x] `docs/design/tagged-semantics.md` §4 — `LIC-2.7p-arith-
      coerce-tagged-1` moved to Resolved. Totals: **28 / 28 / 0**
      (consumer coverage complete).
- [x] `docs/design/tagged-semantics.md` §6 new "TaggedValue arith
      operand coercion" subsection.
- [x] `docs/design/tagged-semantics.md` §8 — ADR 0089 row added.
- [x] `docs/design/0077-phase2-7p-arith-string-coerce.md` — note
      that ADR 0089 extends ADR 0077's surface from static String
      operands to runtime TaggedValue String via the chokepoint.
- [x] `AGENTS.md` — Phase 2.7+ row added; **Phase 2 tagged-semantics
      consumer coverage complete** milestone marker.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative list
(ADR 0068).
