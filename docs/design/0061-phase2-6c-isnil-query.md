# 0061. Phase 2.6c-isnil-query: Inline `t[i] == nil` / `t.k == nil` Non-Trapping Query

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

After 2.6c-tag-hash (ADR 0060) the LIC tracker still carried two
read-side entries:

- **LIC-2.6a-arr-1** — OOB array read traps; Lua spec returns nil.
- **LIC-2.6b-hash-1** — missing hash key read traps; Lua spec
  returns nil.

A complete fix requires *locals widening* (`local x = t[i]; if x
== nil then ...`) so that an arbitrary read can land in a Bool /
Nil-aware slot. That is a substantial phase. But there is a
narrower, very common idiom that does **not** need widening:

```lua
if t[i] == nil then ... end
if t.k ~= nil then ... end
```

The whole `Index == Nil` (or `Ne`) expression collapses to a
boolean test the moment HIR sees it — there is no need to ever
materialise the Index value. That is exactly the contract this
phase delivers.

### The static-fold bug it also fixes

Before this phase, the equality fold in `lower_expr::BinOp` ran
on (lhs_kind, rhs_kind) before any pattern detection. With
`Index` always inferred to `Number` and `Nil` inferred to `Nil`,
heterogeneous-kind fold collapses `t[i] == nil` to
`Bool(false)` unconditionally, throwing the Index away. So:

- `t[1] == nil` → `false` (Lua agrees, but for the wrong reason)
- `t[5] == nil` → `false` (Lua says **true**) — silent miscompile

No existing test covered this, but it was wrong. The fold is now
guarded by an `Index == Nil` pattern detection that runs first.

## Decision

### HIR variant

```rust
HirExprKind::IsNilQuery {
    target: Box<HirExpr>,    // Table-kind
    key:    Box<HirExpr>,    // Number or String kind
}
```

`infer_kind` returns `ValueKind::Bool`. `collect_string_pool`
recurses into both fields so any string-literal key is seeded
into the pool.

### HIR lowering

`lower_expr::BinOp` arm for `Eq | Ne` is augmented with
pattern-match-before-fold:

```rust
let nil_query = match (&lhs_hir.kind, &rhs_hir.kind) {
    (HirExprKind::Index { target, key }, HirExprKind::Nil) => Some((target.clone(), key.clone())),
    (HirExprKind::Nil, HirExprKind::Index { target, key }) => Some((target.clone(), key.clone())),
    _ => None,
};
if let Some((target, key)) = nil_query {
    let query = HirExpr {
        kind: HirExprKind::IsNilQuery { target, key },
        span: expr.span,
    };
    return Ok(match op {
        BinOp::Eq => query,
        BinOp::Ne => HirExpr {
            kind: HirExprKind::UnaryOp { op: UnaryOp::Not, operand: Box::new(query) },
            span: expr.span,
        },
        _ => unreachable!(),
    });
}
// fall through to the existing Eq/Ne fold path
```

`Ne` reuses the existing `UnaryOp::Not` lowering (Bool → Bool
flip via `arith.xori`). The pattern is detected on the lowered
HIR, so subsequent `Index` simplifications would still be
caught.

### Codegen

A new arm of `emit_expr` for `HirExprKind::IsNilQuery` dispatches
on the static key kind:

**Number key (array path)** — guarded f64→i64 + bounds + tag:

```text
key_i = fptosi(key)
length = load(target + 0)
oob = key_i < 1 OR key_i > length
result = scf.if oob:
    yield true
else:
    array_buf = load(target + TABLE_OFF_ARRAY_BUF)
    elem_ptr  = array_buf + (key_i - 1) * ARRAY_ELEM_SIZE
    tag       = load(elem_ptr) i64
    yield tag == TAG_NIL
```

**String key (hash path)** — null buf + non-trapping probe + tag:

```text
hash_buf = load(target + TABLE_OFF_HASH_BUF)
result = scf.if hash_buf == null:
    yield true
else:
    bucket = emit_hash_probe_for_insert(hash_buf, cap, key_str)
    entry_ptr = hash_buf + HASH_OFF_ENTRIES + bucket * HASH_ENTRY_SIZE
    yield scf.if load(entry_ptr) == null:    # missing
        yield true
    else:
        tag = load(entry_ptr + HASH_ENTRY_OFF_VALUE_SLOT)
        yield tag == TAG_NIL
```

The reuse of `emit_hash_probe_for_insert` (instead of
introducing a new probe variant) is the essential simplification
of this phase: that helper already returns a bucket whose key is
either null **or** matches `key_str`, which is exactly what
`IsNilQuery` needs. The trapping `emit_hash_probe_lookup` is
left untouched — the read path continues to use it.

### CA invariants preserved

| Layer    | Change                                                                                          |
|----------|-------------------------------------------------------------------------------------------------|
| Lexer    | None                                                                                            |
| Parser   | None                                                                                            |
| AST      | None                                                                                            |
| HIR      | One new `HirExprKind::IsNilQuery`; one `infer_kind` arm; one pattern detection in `BinOp` Eq/Ne |
| Codegen  | One new `emit_expr` arm for `IsNilQuery`; reuses existing helpers entirely                      |

No existing helper signature changed. No existing test suite
broken. The only HIR rewrite happens *before* the static
heterogeneous-kind fold so the Index value is preserved long
enough to dispatch into the non-trapping path.

## TDD Process

1. **Step 1 — Red.** 11 e2e tests in
   `tests/phase2_6c_isnil_query.rs`. 7 fail (the new behaviour);
   4 already pass (an in-bounds case that happens to fold to
   `false` correctly, the `~=` present-key path, the `and`-
   combined present case, and the `print(t[oob])` regression
   that must keep trapping).
2. **Step 2 — Green.** HIR variant + lowering + visitor + codegen
   arm. All 11 tests pass at 752 (= 741 + 11). No regressions.
3. **Step 3 — ADR + AGENTS + commit.** Single feature commit.

## Alternatives Considered

- **Locals widening (the full fix).** Resolves both LIC entries
  completely (covers `local x = t[i]; if x == nil`, function-
  return nil, etc.). Phase 2.6c-tag-locals will tackle this; it
  needs heterogeneous read paths, Bool/Nil/String/Number-tagged
  Local slots, every reader site updated. Out of scope here —
  blast radius is large, and the inline form covers the most
  common idiom on the way there.
- **Make plain `t[i]` non-trapping** and have callers test the
  tag. Forces every existing trapping read test to be rewritten;
  also widens the read return type from `f64` / `ptr` to a
  tagged value, which is exactly what locals widening will do
  later. Doing it now without locals widening means the *value*
  semantics regress (every `print(t[i])` now needs an
  intermediate dispatch). Rejected.
- **Builtin `is_nil(t, i)`.** Lua-syntax-foreign; users would
  not naturally write it. Rejected.
- **Detect `Index == Nil` only in `cond` position of `if` /
  `while`.** Would limit the rewrite to where it's needed but
  miss `local b = (t.k == nil)`-style binds. The HIR rewrite is
  cheaper than the analysis, so do it eagerly. Rejected.

## Consequences

- ~330 LOC across HIR (~50) + codegen (~250) + visitor (~5).
- 11 new e2e tests; total green at 752 (= 741 + 11).
- **LIC-2.6a-arr-1** transitions pending → **partial** (inline
  `Index == Nil` form resolved; locals-bound reads still pending
  locals widening).
- **LIC-2.6b-hash-1** transitions pending → **partial** (same
  scope).
- Static-fold miscompile of `t[oob_index] == nil` (silently
  yielding `false`) is fixed.
- The decision to reuse `emit_hash_probe_for_insert` for the
  hash side is a small confirmation that probe semantics are
  cleanly factored: trap-on-null lives in
  `emit_hash_probe_lookup` only; insert and IsNilQuery share the
  non-trapping helper.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0060.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `== nil` form: returns true; plain read: exits(1) | **partial (this ADR)** |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values + locals widening |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline `== nil` form: returns true; plain read: exits(1) | **partial (this ADR)** |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number+Nil | partial (ADR 0060) |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | marks Nil tag (key persists) | new (ADR 0060) |

## Out of Scope

- **`local x = t[i]; if x == nil then ...`** — needs locals
  widening so that `x` carries a tag, not a raw `f64`.
- **Function-return-nil** (`local x = f(); if x == nil`) — same
  reason; the function ABI must widen first.
- **Generic any-value `== nil`** (`if some_complex_expr == nil`)
  — once locals widening lands, the same pattern detection
  generalises.
- **Hard tombstone** (LIC-2.6c-tag-hash-1 resolution) — orthogonal;
  remains a follow-up.
- **Iteration** (`pairs(t)` / `ipairs(t)`) — depends on tagged
  reads.
