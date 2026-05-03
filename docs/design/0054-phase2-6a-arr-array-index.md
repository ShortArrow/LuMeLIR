# 0054. Phase 2.6a-arr: Number Array Constructor + Integer Indexing Read

- **Status:** Accepted
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Phase 2.6a-min (ADR 0053) shipped empty tables `{}` and `#t`.
A table you can't put anything in is a useless ceremony — to
make 2.6 actually exercise table semantics we need the
construct/observe pair: a constructor that stores values and
an index expression that loads them back.

The minimum useful slice that covers Lua's array idiom:

```lua
local t = {10, 20, 30}
print(t[1] + t[2] + t[3])   -- 60
print(#t)                   -- 3
```

Hash-keyed access (`t["k"]`, `t.k`), element write
(`t[i] = v`), method syntax, and metatables all defer to
later sub-phases.

## Decision

### Header layout extends the 2.6a-min contract

Same `[length: i64]` at offset 0; element bodies start at
offset 8:

```text
+-------------+
|  i64 length |  ← offset 0  (table pointer)
+-------------+
|  f64 elem₀  |  ← offset 8
|  f64 elem₁  |  ← offset 16
|     …       |
+-------------+
```

`malloc(8 + N*8)` at construction. Each element stored at
offset `8 + i*8`. This preserves the 2.6a-min contract: the
i64 length stays at offset 0 forever; later phases extend
the body region without moving the length.

### Lexer: `[`/`]` become first-class tokens

`LBracket` and `RBracket` join the single-char dispatch.
Long-bracket strings keep priority — the `[` arm only
falls through to the token path when `try_match_long_open`
returns `None`. Existing tests (`[[hello]]` →
`Str("hello")`) keep their semantics; bare `[ x]` now lexes
as `LBracket Ident("x") RBracket Eof`.

### Parser: comma-separated table elements + index suffix

`parse_primary`'s `LBrace` arm now loops:

```text
{               → push `Table([])` and exit if RBrace next
  expr          → parse, push to elems
  ,             → consume, loop (trailing comma OK)
  }             → push `Table(elems)` and exit
}
```

Trailing `,` between the last element and `}` is silently
accepted, matching Lua 5.4. Mismatched closer → `UnexpectedToken`.

`parse_call_suffix` becomes `parse_call_or_index_suffix`
(same fn, broader job): in addition to `(args)` it accepts
`[expr]` to produce `ExprKind::Index { target, key }`. So
`f()[i]`, `t[i][j]`, and `({1,2})[1]` all parse from this
loop. Lua's prefix-expression grammar.

### AST: `ExprKind::Index { target, key }`

Boxed pair — `target` is the prefix expression yielding a
table, `key` the integer index expression. The
`Box<Expr>`-and-`Box<Expr>` shape keeps the ExprKind
variant the same size as `BinOp`.

### HIR: kind dispatch + Number-only enforcement

Table constructor:
- Each element's lowered kind must be `Number`.
  Heterogeneous arrays (e.g. `{1, "two"}`) reject as
  `TypeMismatch` — they need tagged values which haven't
  arrived yet.

Index:
- `target` must lower to `ValueKind::Table`.
- `key` must lower to `ValueKind::Number`.
- Result kind: `Number` (Number-only arrays).

The HIR ExprKind is `Index { target: Box<HirExpr>, key:
Box<HirExpr> }`.

### Codegen: malloc-store + GEP-load + bounds check

Construction emits:
1. `arith.constant i64 (8 + N*8)` for the malloc size
2. `llvm.call @malloc(size)` → table ptr
3. `llvm.store length, ptr` at offset 0
4. For each element: `getelementptr i8, ptr, 8 + i*8` → store
   `f64 elem` at the offset

Index read emits:
1. eval target → `ptr`
2. eval key as `f64`, `arith.fptosi` to `i64`
3. load length from `ptr` (offset 0)
4. **bounds check**: `key < 1 || key > length` →
   `scf.if cond -> exit(1)` with `s_table_oob`
5. compute byte offset `8 + (key-1)*8`
6. `getelementptr i8, ptr, offset` → load `f64` at
   `elem_ptr`

The bounds check uses the same `emit_exit_with_message`
shared helper that 2.7g/2.7h already use.

### OOB → trap (Lua-incompatible, tracked for fix)

Lua spec returns `nil` for out-of-bounds reads. Our static
type system cannot represent a heterogeneous "Number or
nil" return without tagged values, so OOB instead exits
with the message `table index out of bounds`. This is
**security over Lua compatibility** per the project's
preference order (Lua spec > security > speed).

The fix lands when tagged values arrive (Phase 2.6b or
later) — at that point `Index` returns a tagged
"Number-or-nil" and the bounds check changes from "trap"
to "yield nil". This ADR is the load-bearing reference
for that future change.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | `LBracket`/`RBracket` tokens; dispatch in single-char path |
| Parser   | `parse_primary` `{…}` accepts elements + trailing comma; `parse_call_suffix` handles `[…]` index suffix |
| AST      | `ExprKind::Index { target, key }`                   |
| HIR      | `lower_expr` lifts Table empty-only restriction; new `Index` arm with Table-key + Number-key checks; `infer_kind` Index → Number |
| Codegen  | `Types.i8` field; `emit_byte_offset_ptr` + `_dynamic` helpers; `emit_table_bounds_check` helper; `s_table_oob` global; `HirExprKind::Table` stores elements; `HirExprKind::Index` loads with bounds check |

## TDD Process

1. **Red.** 15 e2e tests covering: `#t == 3`, read first /
   middle / last, sum, empty regression, trailing comma,
   direct indexing of literal, computed elements,
   variable-keyed read, OOB trap, zero-index trap,
   non-Number element rejection, non-Number key rejection,
   indexing a non-Table rejection. 14 failed at parse time
   (no `[` / `,` in `{}`); 1 (the `{}` empty regression)
   already passed.
2. **Green.** Lexer tokens, parser comma-loop + index
   suffix, AST `Index`, HIR kind checks, codegen
   element-store / GEP-load / bounds-check. The bounds
   check reuses 2.7g's `emit_exit_with_message` shared
   helper.
3. **Refactor.** None warranted — the new helpers
   (`emit_byte_offset_ptr` / `_dynamic` /
   `emit_table_bounds_check`) factor out the repeated
   GEP + store / GEP + load shapes.

## Alternatives Considered

- **Lua-compatible OOB → nil**. Real correct answer; needs
  tagged values; deferred (see "OOB → trap" above).
- **Heterogeneous element kinds** via runtime tagging.
  Same dependency; deferred.
- **Hash part for string keys** (`t.k`, `t["x"]`). Belongs
  in 2.6b — separate concern, separate codegen.
- **Element write `t[i] = v`**. Belongs in 2.6a.3 — needs
  parser LHS extension and HIR write-side resolution.
- **Capacity tracking + resize**. Lands when write-past-
  end demands it.

## Consequences

- Lexer + parser + AST + HIR + codegen each grow to handle
  the new shapes (~250 LOC total).
- 15 new e2e tests; total green at 684 (669 + 15).
- The `s_table_oob` global is registered for OOB
  diagnostics.
- `Types.i8` joins the type table (used as GEP element
  type for byte-offset arithmetic; will see further reuse
  when struct-shaped types arrive).

## Out of Scope

These items are still deferred. **Items marked Lua-incompatible
return to compatibility once their dependency lands.**

- **Element write `t[i] = v`** — Phase 2.6a.3.
- **Lua-compatible OOB → nil** (Lua-incompatible) —
  pending tagged values.
- **Heterogeneous element kinds** (Lua-incompatible) —
  pending tagged values.
- **Hash part / string keys / `t.k`** — Phase 2.6b.
- **Negative indices** — Lua doesn't have a "from end"
  convention, but our trap currently fires for `t[0]`
  too; matches Lua's "out of array part" semantic.
- **`pairs` / `ipairs`** — pending generic for.
- **Method calls `t:m()`** — pending hash part.
- **Metatables / `setmetatable`** — much later.
- **`type(t)` / `tostring(t)`** — easy adds; defer until
  populated tables can be observed in print/concat.
- **GC** — table memory leaks at process exit, matching
  the existing string runtime.

## Lua-Incompatibility Tracker

Per project policy, Lua-incompatible choices are tracked
explicitly so they can be reverted when their dependency
lands:

| ID | Behaviour | Lua spec | Our behaviour | Trigger to revert |
|----|-----------|----------|---------------|-------------------|
| LIC-2.6a-arr-1 | OOB read | returns nil | exits(1) | tagged values land |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | tagged values land |
| LIC-2.6a-arr-3 | key kinds | any | Number-only | hash part lands (2.6b) |

ADR 0048 (auto-declare globals: chunk-local instead of
`_ENV.k`) and ADR 0043 (Function-kind capture rejected)
are the prior Lua-incompatibility entries — same tracker.
