# 0053. Phase 2.6a-min: Empty Tables `{}` and `#t`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Tables are Lua's central composite type — array, dictionary,
record, and prototype-object all in one. Adding them is the
biggest remaining feature. Done as one mega-phase, the change
would touch every layer at once and resist clean TDD.

This phase ships the **minimum useful**: empty tables `{}`,
the `#t` length operator, and the `ValueKind::Table` plumbing
through the four pipeline layers. Non-empty constructors,
indexing, ipairs, methods, and metatables all defer to
later sub-phases (2.6a.1 onward).

The user-visible win is small but the structural cost is
the bulk of the rework — new ValueKind, new MLIR `!llvm.ptr`
flow, malloc'd reference semantics, kind dispatch in `#`.

## Decision

### Header layout: `[length: i64]` only

```text
+----------+
|  i64 len |   ← table pointer points here
+----------+
| elem 0   |   ← (added in 2.6a.1; not allocated in 2.6a-min)
| elem 1   |
|   ...    |
+----------+
```

The pointer **is** the table value (Lua reference semantics).
The first 8 bytes hold the length; element storage starts at
offset 8.

For the empty form, we malloc 8 bytes and store `0`.
For the populated form (2.6a.1), we'll malloc `8 + N*8`
bytes and store length=N + each element.

Capacity tracking, hash part, and metatable pointer all
defer until they're actually needed:

- **Capacity** lands when `t[i] = v` for `i > #t` arrives (2.6a.4
  or later). Until then, fixed-size means `length == capacity`
  and capacity is implicit.
- **Hash part** lands with string-keyed `t.k` access.
- **Metatables** land with `setmetatable`.

The current header preserves the i64 length at offset 0 — every
future extension keeps that contract so existing code keeps
working.

### `ValueKind::Table` is `!llvm.ptr` everywhere

Type table:

| Layer       | Representation                          |
|-------------|-----------------------------------------|
| AST         | `ExprKind::Table(Vec<Expr>)`            |
| HIR ValueKind | `Table` variant                       |
| HIR Expr    | `HirExprKind::Table(Vec<HirExpr>)`      |
| MLIR slot   | `!llvm.ptr` (alloca holds the heap ptr) |
| MLIR value  | `!llvm.ptr`                             |

Table flows through the existing `String → !llvm.ptr` machinery
unchanged — slot alloca, store, load, return-type dispatch, all
already handle pointer-typed values.

### `#x` is now kind-dispatched in `emit_expr`

Previously the `Len` op always called `strlen` (assuming
String). Now the dispatch happens in `emit_expr`'s
`HirExprKind::UnaryOp` arm where the operand kind is in scope:

```rust
let len_i64 = match kind {
    ValueKind::String => emit_libc_call_i64(context, block, "strlen", &[v], …),
    ValueKind::Table  => emit_load(block, v, types.i64, loc),
    _ => unreachable!("HIR rejects #x for non-String/Table"),
};
```

The old `emit_unary` `Len` arm becomes unreachable — kept for
exhaustiveness but documents the move.

HIR's check widens from `String only` to `String | Table`.

### Param-kind back-inference recognises `{}`

`ast_arg_kind`'s match grows one arm:
`ExprKind::Table(_) => ValueKind::Table`. So
`take({})` calls site refines `take`'s param to Table.
A bare ident arg (`take(x)` where `x = {}`) still falls
through to Number — Ident-name → Table inference is a
documented limitation, deferred to a future phase.

`lower_call`'s arg-vs-param compatibility check grows one
pair: `(Table, Table) => true`. Mirrors the existing
String/Bool/Nil arms.

### Reference semantics

Two locals bound to the same table share storage:

```lua
local a = {}
local b = a  -- alias, same pointer
```

This Just Works because:
- `local b = a` propagates `LocalInfo.func_id` (None for tables)
  and `kind` (Table)
- Codegen for `LocalInit { id: b, value: Local(a_id) }`
  loads the pointer from `slots[a_id.0]` and stores into
  `slots[b_id.0]`
- Both slots hold the same ptr — mutations through one would
  be visible through the other (relevant once write
  indexing arrives in 2.6a.3).

### CA invariants preserved

| Layer    | Change                                                |
|----------|-------------------------------------------------------|
| Lexer    | `LBrace` / `RBrace` tokens; dispatch in single-char path |
| Parser   | `parse_primary` arm for `LBrace`; AST `ExprKind::Table` |
| AST      | `ExprKind::Table(Vec<Expr>)`                          |
| HIR      | `ValueKind::Table`; `HirExprKind::Table`; `coerce_to_string` rejects Table; `infer_kind` arm; `ast_arg_kind` arm; `lower_call` kind-compat arm; `#` widens to String\|Table |
| Codegen  | `param_mlir_type` / `ret_mlir_types` / `emit_alloca_slot_for_kind` / `kind_to_mlir_type` Table arms (all `!llvm.ptr`); `s_typename_table` global; `HirExprKind::Table` materialises malloc + length-store; `#` dispatches in `emit_expr` |

## TDD Process

1. **Red.** 6 e2e tests covering compile-and-run, `#t`,
   multi-instance, literal-arg-to-fn, alias semantics, and a
   `#"hello"` regression. 5 failed (parser rejected `{`); 1
   passed (regression).
2. **Green.** Lexer tokens → AST → HIR → codegen, in that
   order. The non-exhaustive-match cascade across codegen
   was mechanical — every `match kind { … }` and `match
   &expr.kind { … }` on `ValueKind` / `HirExprKind` got a
   Table arm. The `#` dispatch refactor (move from
   `emit_unary` to `emit_expr`) was the largest sub-change.
3. **Refactor.** None warranted — the changes are localised
   and the new arms mostly mirror the existing String arms.

## Alternatives Considered

- **`[length: i64, capacity: i64]` 16-byte header** from
  the start. Cleaner for 2.6a.4's growth semantics, but
  capacity is unused until that phase actually arrives.
  Rejected — minimum scope wins; capacity is additive
  (extend the header at offset 8, leave length at 0).
- **`Table { len, ptr → buffer }` two-allocation form**.
  Better for stable outer-pointer alias even when buffer
  reallocs. Premature; rejected.
- **Lua-reference full layout** (array + hash + metatable).
  Rejected — too many fields are zero or unused for
  2.6a-min.
- **Skip the `#t` arm**, let it surface as a TypeMismatch
  for now. Defeats the test surface; would force a partial
  feature. Rejected.

## Consequences

- AST + HIR + codegen each pick up ~1 enum variant.
- ~150 LOC added net (codegen carries most of the weight
  with kind-dispatch arms).
- 6 new e2e tests; total green at 669 (663 + 6).
- The `s_typename_table` global is registered (used by a
  future `type(t)` extension; not yet wired through HIR's
  `Type` exception list — that lands in 2.6a.1 or later
  when populated tables can be observed).

## Out of Scope

- **Non-empty constructors** `{1, 2, 3}` / `{ x = 1 }` —
  Phase 2.6a.1 onward.
- **Indexing read** `t[i]` / `t.k` — 2.6a.2 / 2.6a.4.
- **Indexing write** `t[i] = v` — 2.6a.3.
- **`pairs` / `ipairs`** — pending `for k, v in …` lowering.
- **Method syntax** `t:m()` — pending.
- **Metatables / `setmetatable`** — much later.
- **`type(t)` / `tostring(t)`** — easy to add but defer until
  populated tables exist (so the test surface is meaningful).
- **Nil keys, weak tables, finalizers** — far out of scope.
- **GC** — the malloc'd region leaks at process exit,
  matching Phase 2.7a's string handling. A separate phase
  introduces a runtime arena or Bohem GC.
- **Ident-name → Table param-kind inference** — falls through
  to Number today; lifts when local-resolution lands in the
  pre-scan.
