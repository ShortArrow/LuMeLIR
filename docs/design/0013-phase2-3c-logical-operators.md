# 0013. Phase 2.3c: Short-Circuit `and` / `or` / `not`

- **Status:** Accepted
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.3b (ADR 0012) finished `if`/`while` and the `emit_truthiness`
helper. Without `and`/`or`/`not`, conditions can only be a single
comparison or a literal — anything more elaborate needs nested `if`s.
Phase 2.3c adds the three logical operators so that `if not done and
ready then ...` and similar idioms work.

`emit_truthiness` (defined per ADR 0012) is reused: every kind reduces
to an `i1` truthiness, then the operator works on that bit.

## Decision

### 1. AST

```rust
pub enum BinOp { ..., And, Or }
pub enum UnaryOp { Neg, Not }
```

`and`/`or` reuse the existing `BinOp` machinery for parsing, but their
codegen path bypasses `emit_binop` (they short-circuit, so the right
operand may not be evaluated).

### 2. Precedence (Lua 5.4 §3.4.8)

- `or` — `PREC_OR = 5` (lowest among the operators we support)
- `and` — `PREC_AND = 6`
- comparison — `PREC_CMP = 8` (existing)
- `not` — `PREC_UNARY = 12` (same level as unary `-`)

Both `and` and `or` are left-associative.

### 3. HIR rules

- `not a`: any `ValueKind` operand. Result kind is **always `Bool`**.
- `a and b` / `a or b`: both sides must share a `ValueKind`. Result
  kind matches both operands. Heterogeneous (e.g. `1 and true`) is
  `HirError::TypeMismatch` — Lua's value-preserving semantics for
  mixed kinds requires a runtime tag we don't yet have.

`infer_kind` extension:

```rust
HirExprKind::UnaryOp { op: UnaryOp::Not, .. } => ValueKind::Bool,
HirExprKind::BinOp { op: BinOp::And | BinOp::Or, lhs, .. } =>
    infer_kind(lhs, locals),  // same as rhs by lower-time check
```

### 4. Codegen

#### `not a`

`emit_unary` gains a `UnaryOp::Not` arm:

1. `truth = emit_truthiness(operand_value, kind)` — reuse 2.3b helper.
2. `arith.constant 1 : i1` (the `true` constant).
3. `arith.xori truth, true_const : i1` flips the bit. Result is
   always `Bool`.

A single arithmetic op, no control flow.

#### `a and b` / `a or b`

`emit_short_circuit` builds an `scf.if` in **expression form** —
`result_types` non-empty, both regions terminating with
`scf.yield value`:

```mlir
%lhs = ...
%cond = <truthiness of lhs>
%result = scf.if %cond -> (T) {
  // For `and`: evaluate rhs and yield it
  // For `or`:  yield lhs
} else {
  // For `and`: yield lhs
  // For `or`:  evaluate rhs and yield it
}
```

`T` is determined by `kind_to_mlir_type(kind, types)`:

| `ValueKind` | MLIR type   |
| ----------- | ----------- |
| `Number`    | `f64`       |
| `Bool`      | `i1`        |
| `Nil`       | `i1`        |

`emit_expr`'s `BinOp::And/Or` case is special-cased to call
`emit_short_circuit` instead of the regular `emit_binop` (the
ordinary path eagerly evaluates both operands).

The lhs `Value` computed in the parent block is referenced inside one
of the inner regions for `scf.yield`. The same dominance argument
that made `transmute_slots` sound in 2.3b applies — outer Values
dominate inner regions. If Rust's borrow checker rejects, a
`transmute_value` helper of the same shape can be added.

### 5. Lexer

`Keyword` gains `And`, `Or`, `Not`. `from_lexeme` matches on the
spelling. The keyword post-processing path (existing since Phase 2.0)
classifies identifier scans as keyword tokens before the parser sees
them.

## Alternatives Considered

- **Heterogeneous `1 and true` allowed.** Lua's value-preserving
  return rule produces a value whose kind depends on runtime
  truthiness — needs a dynamic type tag. Defer.
- **Coerce `and`/`or` results to `Bool`.** Type-clean but diverges
  from Lua, and `cond and a or b` (a common ternary idiom) becomes
  even less useful. Rejected.
- **`not a` as `arith.cmpi eq, truth, 0 : i1`.** Equivalent
  semantics but one extra constant; `xori` is more direct.
- **Short-circuit via `cf.cond_br` + PHI.** `scf.if` expression form
  avoids manual SSA management — the `result_types` mechanism already
  performs the join.

## Consequences

- `Keyword` +3 (`And`, `Or`, `Not`).
- `BinOp` +2, `UnaryOp` +1.
- `PREC_OR`, `PREC_AND` constants added to the parser.
- `emit_short_circuit` and `kind_to_mlir_type` join the codegen helpers.
- This is the first use of `scf.if`'s expression form (non-empty
  `result_types`).
- Phase 2.3d (`for` loops) and Phase 2.4 (functions) are independent
  of this work and can proceed once 2.3c lands.

## Out of Scope (still deferred)

- Heterogeneous `and`/`or` (`1 and true`) → Phase 2.4+ (dynamic types).
- `for` loops → Phase 2.3d.
- Function definitions, `return`, `break` → Phase 2.4+.
- Tables / metatables / GC → Phase 2.5+.
