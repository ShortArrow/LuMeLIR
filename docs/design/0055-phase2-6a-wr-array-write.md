# 0055. Phase 2.6a-wr: Number Array Element Write `t[i] = v`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Phase 2.6a-arr (ADR 0054) shipped construction (`{1, 2, 3}`)
and read indexing (`t[i]`). Tables are observable but not
mutable — useless as a real data structure. The smallest
addition that completes minimal CRUD is in-bounds write:
`t[i] = v` for `1 ≤ i ≤ #t`. This unlocks bubble-sort-class
algorithms and the standard "fill an array in a loop" idiom.

Lua's full write semantics include grow (`t[#t+1] = v`
extends length) and holes (`t[5] = v` for length-3 tables
creates a sparse array). Both require capacity tracking and
defer to a later sub-phase. OOB writes trap — same security
trade-off as 2.6a-arr's read OOB.

## Decision

### Parser: `parse_stmt` fallthrough learns expr-then-equals

The pre-2.6a-wr fallthrough wrapped any unparsed prefix
into `ExprStmt` immediately. The new path:

```rust
_ => {
    let expr = self.parse_expr(0)?;
    if matches!(self.peek().kind, TokenKind::Equals) {
        let eq_tok = self.bump().clone();
        let value = self.parse_expr(0)?;
        return match expr.kind {
            ExprKind::Index { target, key } => {
                let span = Span::new(target.span.start, value.span.end);
                Ok(Stmt::new(
                    StmtKind::IndexAssign { target: *target, key: *key, value },
                    span,
                ))
            }
            _ => Err(ParseError::UnexpectedToken {
                actual: TokenKind::Equals,
                offset: eq_tok.span.start,
            }),
        };
    }
    let span = expr.span;
    Ok(Stmt::new(StmtKind::ExprStmt(expr), span))
}
```

Pratt's expression precedence already excludes `=`, so
`parse_expr` halts before the equals sign — the post-check
sees a clean separator. Future phases adding `t.k = v` /
`obj.f.s = v` reuse the same fallthrough by matching new
lvalue shapes (`ExprKind::Field`, etc.).

### AST + HIR mirror variants

```rust
// AST
StmtKind::IndexAssign { target: Expr, key: Expr, value: Expr }

// HIR
HirStmtKind::IndexAssign { target: HirExpr, key: HirExpr, value: HirExpr }
```

HIR's `lower_stmt` arm enforces the same kind constraints as
the read-side `Index` expr arm (target Table, key Number,
value Number) — heterogeneous element kinds defer until
tagged values arrive (LIC-2.6a-wr-3).

### Codegen: read path mirror

`emit_stmt`'s new `IndexAssign` arm reuses every helper
introduced in 2.6a-arr:

- `emit_table_bounds_check(key_i, length_i)` — same predicate
  `key < 1 || key > length` triggers the same `s_table_oob`
  trap.
- `emit_byte_offset_ptr_dynamic(target_ptr, header_offset)` —
  same `8 + (key-1)*8` byte offset.
- `emit_store` — replaces the read path's `emit_load`.

The shared layout contract (i64 length at offset 0, f64
elements from offset 8) means write and read see the same
addresses for the same indices.

### Reference semantics fall out

`local b = a; b[1] = 99; print(a[1])` prints `99`. Tables
are heap-pointer values — `local b = a` copies the pointer
into a new slot, both slots reference the same heap region.
A write through either slot is observable through both.
The e2e test pins this.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | `parse_stmt` fallthrough post-check; AST `IndexAssign` |
| AST      | `StmtKind::IndexAssign { target, key, value }`      |
| HIR      | `HirStmtKind::IndexAssign`; `lower_stmt` arm with kind checks; pre-scan `visit_stmt` arms (×2) |
| Codegen  | `emit_stmt` arm reusing `emit_table_bounds_check`, `emit_byte_offset_ptr_dynamic`, `emit_store` |

## TDD Process

1. **Red.** 11 e2e tests covering: existing-index write,
   self-reference RHS, loop fill, OOB write trap, grow trap
   (LIC-2.6a-wr-2), zero-index trap, alias-write reference
   semantics, value-kind / key-kind / target-kind static
   rejections, and a 2.6a-arr read regression. 10 failed
   at parse time; 1 (the read regression) passed.
2. **Green.** Five mechanical changes — AST variant, parser
   post-check + `strip_span_stmt` arm, HIR variant + lower
   arm + two pre-scan visit arms, codegen `emit_stmt` arm.
   All 11 tests pass at 695 (684 + 11).
3. **Refactor.** None warranted — the new arm reuses three
   helpers verbatim from the read path. The "same byte-
   offset calculation in read and write" is now visible
   enough that a future phase could extract it, but with
   only two call sites the rule of three says wait.

## Alternatives Considered

- **Generalize `parse_assign` to accept `Ident[…]` LHS**
  with extended lookahead. Works for the single-target case
  but doesn't compose with future `t.k = v` / `obj.f.s = v`.
  Rejected — the post-`parse_expr` check generalises better.
- **Multi-target `t[1], t[2] = a, b`**. Would need
  `IndexAssignMulti` mirroring 2.1a's `AssignMulti`. Defer
  until a real use case.
- **Lua-compatible grow on `t[#t+1] = v`** in this phase.
  Requires capacity tracking + reallocation — a separate
  structural change. Defer.

## Consequences

- ~150 LOC across parser/AST/HIR/codegen + 200 LOC of tests.
- 11 new e2e tests; total green at 695 (684 + 11).
- All three byte-offset helpers (`emit_byte_offset_ptr`,
  `_dynamic`, `emit_table_bounds_check`) now have read +
  write call sites — design pre-paid in 2.6a-arr.
- The expr-then-equals fallthrough gives future field-access
  writes a ready home.

## Out of Scope

- **Grow on write** (`t[#t+1] = v`) — pending capacity tracking.
- **Holes** (`t[5] = v` on length-3) — pending capacity tracking.
- **`t.k = v` / `t["k"] = v`** — pending hash part (Phase 2.6b).
- **Multi-target index assign** — pending demand.
- **Heterogeneous value kinds** — pending tagged values.
- **Compound assignment** (`t[i] += v`) — Lua doesn't have it.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0054. Newly added: LIC-2.6a-wr-1/2/3.

| ID | Behaviour | Lua spec | Our behaviour | Trigger to revert |
|----|-----------|----------|---------------|-------------------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | tagged values land |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | tagged values land |
| LIC-2.6a-arr-3 | key kinds | any | Number-only | hash part lands (2.6b) |
| LIC-2.6a-wr-1 | OOB write | creates a hole | exits(1) | tagged values land |
| LIC-2.6a-wr-2 | grow write `t[#t+1]=v` | extends length | exits(1) | capacity tracking lands |
| LIC-2.6a-wr-3 | value kinds | heterogeneous | Number-only | tagged values land |

The earlier LIC entries (ADR 0048 globals: chunk-local
instead of `_ENV.k`; ADR 0043 Function-kind capture
rejected) belong on this same tracker — pending a
roll-up that consolidates all LIC IDs in one canonical
location (post-Phase-2.6 cleanup).
