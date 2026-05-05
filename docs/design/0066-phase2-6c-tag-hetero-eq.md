# 0066. Phase 2.6c-tag-hetero-eq: IsNil Unification + Local-Local TaggedValue Eq

- **Status:** Accepted
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

Codex review on Phase 2.6c-tag-hetero-fix (ADR 0065) closed
gave a 5-axis (TDD / FP / CA / security / docs) read of the
codebase and recommended:

1. Resolve `LIC-2.6c-tag-hetero-eq-1` — the last open hole in
   TaggedValue equality — by adding a runtime tag dispatch for
   `Local(TaggedValue) == Local(TaggedValue)`.
2. As a Tidy First step before that feature lands, collapse
   `HirExprKind::IsNilQuery` (ADR 0061) and `HirExprKind::IsNilLocal`
   (ADR 0063) into a single `HirExprKind::IsNil(Box<HirExpr>)`.
   Both variants share the "tagged-value source → Bool, never
   traps" semantics; keeping them apart is HIR枝分かれ that
   future hetero consumers would compound.

This ADR delivers both as one phase, with the Tidy First in a
separate refactor commit ahead of the feature commit.

User-visible change:
```lua
local t = {"a", "a"}
local x = t[1]
local y = t[2]
if x == y then print("equal") end       -- ADR 0066: prints "equal"
                                        -- (pre-ADR-0066 would trap on
                                        -- the Number-only extract path)
```

## Decision

### Phase A — Tidy First: `IsNil(Box<HirExpr>)`

A single HIR variant replaces the pair:

```rust
pub enum HirExprKind {
    // ... existing
    IsNil(Box<HirExpr>),  // operand: Index { … } | Local(TaggedValue)
}
```

`lower_expr::BinOp Eq/Ne` runs one match that produces the
unified shape from either pattern:

```rust
let nil_operand = match (&lhs.kind, &rhs.kind) {
    (HirExprKind::Index { .. }, HirExprKind::Nil) => Some(lhs.clone()),
    (HirExprKind::Nil, HirExprKind::Index { .. }) => Some(rhs.clone()),
    (HirExprKind::Local(LocalId(idx)), HirExprKind::Nil)
        if matches!(self.locals[*idx].kind, ValueKind::TaggedValue) => Some(lhs.clone()),
    (HirExprKind::Nil, HirExprKind::Local(LocalId(idx)))
        if matches!(self.locals[*idx].kind, ValueKind::TaggedValue) => Some(rhs.clone()),
    _ => None,
};
```

Codegen's IsNil arm dispatches on `operand.kind`:
- `Local(LocalId(idx))` — load tag at `slots[idx]+0`, compare
  with `TAG_NIL`. (Two MLIR ops; inline.)
- `Index { target, key }` — call `emit_isnil_index`, which holds
  the original ADR 0061 lowering verbatim (Number-key bounds
  + array tag, String-key probe + bucket tag). Extracted as a
  helper because the body is large and would otherwise dominate
  the new IsNil arm.

Behaviour is preserved exactly. 794 tests still green after
the Tidy First commit.

### Phase B — Local-Local TaggedValue Eq runtime dispatch

`emit_tagged_eq_runtime_dispatch` (introduced in ADR 0065) used
to short-circuit `(Some, Some) => return Ok(None)` on the
both-tagged case, leaving the comparison to fall through to the
trapping Number-only path. ADR 0066 routes that case into a new
`emit_tagged_eq_local_local(slot_lhs, slot_rhs)` helper:

```text
lhs_tag = load(slot_lhs + 0)
rhs_tag = load(slot_rhs + 0)
result = scf.if (lhs_tag == rhs_tag):
    scf.if (tag == TAG_NIL):       yield true
    elif (tag == TAG_NUMBER):      yield (cmpf Oeq, lhs_payload f64, rhs_payload f64)
    elif (tag == TAG_BOOL):        yield (cmpi  Eq, lhs_payload i64, rhs_payload i64)
    elif (tag == TAG_STRING):      yield (strcmp(lhs_p, rhs_p) == 0)
    else (Function/Table reserved): yield false
else:                              yield false
```

`Ne` continues to wrap with `xori(eq, true)` on the caller side.

Lua semantics covered:
- Same-kind same-value → true
- Same-kind different-value → false (per-kind predicate)
- Different-kind → false
- Both nil → true
- Function / Table tag (4 / 5) → defensive false, since those
  payloads aren't yet lowered

### CA invariants preserved

| Layer    | Change                                                                                       |
|----------|----------------------------------------------------------------------------------------------|
| Lexer    | None                                                                                         |
| Parser   | None                                                                                         |
| AST      | None                                                                                         |
| HIR      | -2 +1 variants (`IsNilQuery` / `IsNilLocal` → `IsNil`). Pattern detection in `lower_expr` collapses to one match. |
| Codegen  | New `emit_isnil_index` helper holds the ADR 0061 body. New `emit_tagged_eq_local_local` helper. `emit_tagged_eq_runtime_dispatch` route Both-Local into the new helper. |

## TDD Process

1. **Step 0 — Tidy First.** Mechanical IsNil unification.
   `cargo test` stays at 794. Separate
   `refactor(hir,codegen): unify IsNilQuery/IsNilLocal into IsNil`
   commit.
2. **Step 1 — Red.** 10 e2e tests in
   `tests/phase2_6c_tag_hetero_local_local_eq.rs`. Seven fail
   (Both-Local Eq paths), three already pass (regression
   coverage).
3. **Step 2 — Green.** `emit_tagged_eq_local_local` + dispatch
   route. All 10 tests pass; 794 + 10 = 804 green.
4. **Step 3 — ADR + AGENTS + commit.**

## Alternatives Considered

- **HIR-reject Both-Local TaggedValue Eq.** Avoids runtime
  dispatch but pushes a `tostring(x) == tostring(y)` workaround
  onto users. Rejected.
- **Trap on Both-Local TaggedValue Eq when tags differ.**
  Lua-incompatible; spec says `1 == "1"` is false, not error.
  Rejected.
- **Generalise the four-way tag dispatch into a generic helper
  upfront.** Currently exactly one call site. Defer until a
  second consumer requires the same shape — Codex review
  flagged TaggedValue dispatch helper extraction as an
  upcoming Tidy First but deferred it past this phase.
- **Single ADR for Tidy First only, separate ADR for the
  feature.** Plausible, but the two changes share a Codex-
  review motivation and the Tidy First is what makes the
  feature commit small. One ADR keeps the rationale together;
  the two commits keep the diff reviewable.

## Consequences

- HIR: net **-1** variant. `lower_expr::BinOp Eq/Ne` pattern
  detection is one match instead of two parallel ones.
- Codegen: `emit.rs` adds `emit_isnil_index` (~300 LOC,
  extracted from the old IsNilQuery arm with no logic change)
  and `emit_tagged_eq_local_local` (~190 LOC, new). The
  existing `emit_tagged_eq_runtime_dispatch` shrinks slightly
  on the Both-Local fast path and grows on the new route.
- Tests: 10 new e2e tests in
  `tests/phase2_6c_tag_hetero_local_local_eq.rs`. Total green
  at **804**.
- **`LIC-2.6c-tag-hetero-eq-1` → resolved.** All TaggedValue
  equality cases (Local-Literal in ADR 0065, Local-Local now)
  match Lua spec.
- ADR 0061 / 0063 references to `IsNilQuery` / `IsNilLocal`
  are still meaningful as historical context. The semantics
  carry over to `IsNil`; only the variant name changed.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0065.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `print(t[oob])` prints nil; non-print uses still extract f64 | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number/Bool/String/Nil supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number/Bool/String supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline print: nil; non-print uses still trap on extract | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number/Bool/String/Nil-delete supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | resolved (ADR 0062) |
| LIC-2.6c-tag-locals-1 | `type(x)` for widened local | runtime dispatch on actual tag | static "number" | new (ADR 0063) |
| LIC-2.6c-tag-hetero-fn-tbl-1 | Function/Table table values | accepted | rejected at HIR | new (ADR 0064) |
| LIC-2.6c-tag-hetero-eq-1 | `==`/`~=` between two `TaggedValue` locals | runtime tag-aware | runtime tag dispatch with per-kind compare | **resolved (this ADR)** |
| LIC-2.6c-tag-hetero-inline-1 | inline `print(t[k])` for hetero values | prints typed value or "nil" | runtime tag dispatch | resolved (ADR 0065) |

## Out of Scope

- **Tagged dispatch helper extraction in `emit.rs`** — Codex
  review's secondary Tidy First item. Defer until a third
  hetero consumer (currently `print` and `eq`) emerges.
- **Other `Index` consumers (arith, cmp outside `==`,
  `tostring`, `type`)** for hetero values — same Tidy First
  unlock. `type(x)` is `LIC-2.6c-tag-locals-1`; arith / cmp
  trap appropriately for now.
- **Function-return TaggedValue widening** —
  `local x = f()` where `f` returns nil/heterogeneous.
- **Function/Table values in tables** — closure-escape /
  ucast / cycle work.
- **Tagged-semantics consolidation doc** — Codex review's
  documentation suggestion (`docs/design/tagged-semantics.md`
  one-pager). Defer until ADR 0067-style follow-ups stabilise.
