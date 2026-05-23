# 0063. Phase 2.6c-tag-locals: Number-MaybeNil Locals Widening

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0061 made the *inline* form `if t[i] == nil then ...`
Lua-correct by detecting `Index == Nil` before the static fold
and lowering to a non-trapping `IsNilQuery`. ADR 0062 did the
structural cleanup of `t.k = nil` (hard tombstone). What
remained from the LIC tracker was the **locals form**:

```lua
local x = t[5]
if x == nil then ... end       -- previously: trap at `local x = t[5]`
```

The trapping read path (`emit_value_slot_check_number`) on
`HirExprKind::Index` always assumed every read produces a live
Number. To let `x` carry "Number-or-Nil" we need locals
widening: a 16-byte tagged slot (`{i64 tag, f64 value}`) that
the read site can populate without trapping.

Locals widening at full strength (`MaybeNil(Bool)`,
`MaybeNil(String)`, function-return widening, full reflective
`type(x)` etc.) is sub-phase territory. This ADR scopes the
**minimum** that resolves the LIC entries:

- `MaybeNilNumber` only — the inner kind is always Number.
- The trigger is `LocalInit { value: Index }` and `Assign` to
  an existing MaybeNilNumber-kind local. Other expression
  contexts keep their existing trapping `Index` codegen.
- Reading the local from non-IsNil contexts still traps on Nil
  (`x + 1`, `print(x)`) — this matches Lua's "nil arithmetic
  is an error" semantics. `if x == nil then` now reaches the
  trapping path under the false branch only when the runtime
  tag actually says Number.

## Decision

### Existing `Index` path is preserved

The single most important property: `infer_kind(Index)`
**still returns `Number`**, the codegen for `HirExprKind::Index`
**still traps on Nil**. Existing tests and idioms like
`print(t[i])` / `t[i] + 1` / `if t[i] == nil` (handled by ADR
0061) keep working unchanged.

The widening is opt-in via *statement context*: only
`LocalInit` / `Assign` triggers it.

### New types

```rust
// src/hir/mod.rs
pub enum ValueKind {
    Number, Bool, Nil, Function(usize), String, Table,
    /// Phase 2.6c-tag-locals (ADR 0063). 16-byte tagged slot
    /// `{i64 tag, f64 value}` that may carry Number or Nil.
    MaybeNilNumber,
}

// src/hir/ir.rs
pub enum HirExprKind {
    // ... existing
    /// Non-trapping tagged read of a table cell. Produced only
    /// by `lower_stmt(LocalInit | Assign)` from a plain
    /// `HirExprKind::Index`. Consumed inline by codegen's
    /// `emit_local_init_tagged` — it never surfaces in
    /// expression context (`emit_expr` panics on it).
    IndexTagged { target: Box<HirExpr>, key: Box<HirExpr> },
    /// Local-side counterpart to `IsNilQuery`. Detected from
    /// `Local(MaybeNilNumber) == Nil` (or `~= nil`). Reads the
    /// slot's tag at offset 0 and returns `tag == TAG_NIL` as
    /// i1 — never traps.
    IsNilLocal { local_id: LocalId },
}
```

`MaybeNilNumber` is intentionally a concrete variant rather
than `MaybeNil(Box<ValueKind>)` so `ValueKind` keeps `Copy` and
the existing 180+ use sites are unaffected. When the next
sub-phase needs `MaybeNil(Bool)` etc., the variant can be
generalised in one Tidy First commit.

### HIR rewriting

`lower_stmt(LocalInit)` and `lower_assign_target` now route the
incoming value through `widen_index_for_local_init`, which
rewrites `HirExprKind::Index { target, key }` into
`IndexTagged { target, key }`. `infer_kind` then picks
`MaybeNilNumber` and the local declares with that kind.

The `BinOp::Eq | BinOp::Ne` pattern detection in `lower_expr`
gains a parallel branch: `Local(MaybeNilNumber) == Nil` lowers
to `IsNilLocal { local_id }`. `Ne` continues to wrap the result
in `UnaryOp::Not` as ADR 0061 already did for `IsNilQuery`.

The existing arithmetic / comparison type checks switch from
`lk == ValueKind::Number` to `is_number_compatible(lk)` so
`MaybeNilNumber + Number` parses without HIR error. The actual
trap-on-Nil materialises at the codegen Local read.

### Codegen

| Site                      | Behaviour                                                                    |
|---------------------------|------------------------------------------------------------------------------|
| `emit_alloca_slot_for_kind` | `MaybeNilNumber` → `alloca i64 × 2` (16 bytes, 8-byte aligned)              |
| `HirExprKind::Local`       | `MaybeNilNumber` → `emit_value_slot_check_number` + load f64 at offset +8 |
| `emit_stmt(LocalInit/Assign)` for `MaybeNilNumber` dst | dispatch on value: `IndexTagged` → `emit_local_init_tagged`; `Local(MaybeNilNumber)` → 16-byte copy; else → `emit_value_slot_store_number` |
| `HirExprKind::IsNilLocal`  | load tag at slot+0 and compare with `TAG_NIL`; never traps                  |
| `HirExprKind::IndexTagged` (in `emit_expr`) | `unreachable!()` — only `emit_local_init_tagged` consumes it |

`emit_local_init_tagged` is the inverted twin of the
`IsNilQuery` codegen: same bounds / probe / null-buf shape,
but its result lands as `{tag, value}` at `dst_slot` instead of
an i1 truth value.

### CA invariants preserved

| Layer    | Change                                                                                                                                           |
|----------|--------------------------------------------------------------------------------------------------------------------------------------------------|
| Lexer    | None                                                                                                                                             |
| Parser   | None                                                                                                                                             |
| AST      | None                                                                                                                                             |
| HIR      | Three new types (`ValueKind::MaybeNilNumber`, `HirExprKind::IndexTagged`, `HirExprKind::IsNilLocal`); rewrite in `lower_stmt`; pattern detection in `lower_expr`; arithmetic type-check helper |
| Codegen  | Slot alloca size for MaybeNilNumber; Local read tag-check path; `emit_local_init_tagged`; `IsNilLocal` arm; new visitor arms                    |

## TDD Process

1. **Step 1 — Red.** 10 e2e tests in
   `tests/phase2_6c_tag_locals.rs`. 5 fail outright, 4 unrelated
   passing (in-bounds reads, present-key, regression test).
2. **Step 2 — Green.** HIR variants + lowering + codegen. All
   10 pass at 769 (= 759 + 10). 759 baseline tests pass without
   any regression.
3. **Step 3 — ADR + AGENTS + commit.** Single feature commit.

## Alternatives Considered

- **`infer_kind(Index) → MaybeNilNumber` everywhere.** Would
  remove the statement-context dispatch but turns every
  existing `print(t[i])` / `t[i] + 1` site into a tagged read
  with extract-or-trap. The behaviour is equivalent (still
  traps on Nil), but the semantics shift could surprise the
  next reader and the blast radius spans every callee that
  takes `kind: ValueKind`. The opt-in form via `IndexTagged`
  is reversible and surgically scoped. Rejected.
- **`MaybeNil(Box<ValueKind>)` generic variant.** Forces drop
  of `Copy` on `ValueKind`. With ~180 use sites currently
  passing kind by value, the mechanical churn is significant
  for zero current benefit (we only need MaybeNilNumber).
  Defer until a second `MaybeNil`-flavoured kind is actually
  needed. Rejected.
- **Function-return widening in this phase.** `local x = f()`
  where `f` returns nil should also widen `x`. That requires
  the function ABI to return a 16-byte tagged value, which
  cuts across `func.func` signature lowering and is genuinely
  separate work. Out of scope for this sub-phase; deferred to
  Phase 2.6c-tag-locals-fn.
- **Reuse `IsNilQuery` for the local case** (rename to a
  generic `IsNil(Box<HirExpr>)`). Cleaner long-term, but the
  rename touches every IsNilQuery codegen path. Keep both
  variants in parallel here; a Tidy First commit can unify
  them after the next-phase shape is clearer (e.g. once
  function-return widening lands and produces yet a third
  IsNil source).

## Consequences

- ~80 LOC HIR + ~250 LOC codegen + ~150 LOC tests. Total feature
  diff ~480 LOC plus this ADR.
- 10 new e2e tests; total green at 769 (= 759 + 10).
- **LIC-2.6a-arr-1 → resolved** for the locals form.
- **LIC-2.6b-hash-1 → resolved** for the locals form.
- A widened MaybeNilNumber local consumes 16 bytes of stack
  per slot (vs 8 for plain Number). Only paid on locals that
  the rewrite triggers on.
- `type(x)` on a MaybeNilNumber static-dispatches to
  `"number"` regardless of the actual runtime tag. Logged as
  a follow-up LIC; the widening tests do not exercise this
  path.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0062.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `==nil`: true; locals form: nil-tagged then trap or IsNil | **resolved (this ADR + 0061)** |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values + locals widening |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline `==nil`: true; locals form: nil-tagged then trap or IsNil | **resolved (this ADR + 0061)** |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number+Nil | partial (ADR 0060) |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | resolved (ADR 0062) |
| LIC-2.6c-tag-locals-1 | `type(x)` for widened local | runtime dispatch on actual tag | static "number" | new (this ADR) |

## Out of Scope

- **Function-return widening** — `local x = f()` where `f`
  returns nil. Needs function ABI updates. Phase
  2.6c-tag-locals-fn.
- **`MaybeNilBool` / `MaybeNilString` / etc.** — heterogeneous
  Local widening. Requires generalising the `MaybeNilNumber`
  variant.
- **Heterogeneous table values** (Bool/String/Function/Table
  in array slots / hash entries). Pending tag-space expansion.
- **Runtime `type(x)` dispatch on widened locals** — needs
  scf.if + tag check at the type-builtin call site. Logged as
  LIC-2.6c-tag-locals-1.
- **Tidy First: unify `IsNilQuery` and `IsNilLocal`** —
  candidate refactor after a third `IsNil`-flavoured source
  emerges.
- **`local x = ...; x = "hello"` kind override** — current
  HIR rejects via `resolve_or_declare_target`. Lua spec is
  permissive (dynamic). Pending dynamic-typing work.
- **Iteration `pairs` / `ipairs`** — depends on full
  heterogeneous reads.
