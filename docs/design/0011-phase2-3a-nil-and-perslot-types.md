# 0011. Phase 2.3a: `nil` Literal, Per-Slot Type Tracking, and Heterogeneous `==`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

ADR 0010 (Phase 2.2b) explicitly deferred three Lua semantic features
to Phase 2.3+. Each is a prerequisite for `if`/`while` (Phase 2.3b)
and the `and`/`or`/`not` short-circuit operators (Phase 2.3c):

1. `local b = true` is rejected because all stack slots are `f64`.
2. There is no `nil` literal — Lua's central absence-of-value cannot
   be expressed.
3. Heterogeneous `==` (e.g. `1 == nil`, `1 == true`) is currently a
   `HirError::TypeMismatch`. Lua's `==` returns `false` for any
   different-typed pair.

Phase 2.3a is the foundation that fixes all three at once and makes
the static value model `{ Number, Bool, Nil }` complete. Once it lands,
2.3b can introduce control flow whose condition expressions consume
values of any of these kinds via a `truthiness(value, kind) -> i1`
helper that 2.3a does *not* yet need to write.

The split keeps each ADR roughly the same scale as ADRs 0009 and 0010
and matches the established 4–6 commit cadence per phase.

## Decision

### 1. `nil` keyword and AST

`Keyword::Nil` is added to the lexer (post-processed from the `nil`
identifier lexeme). The AST gains `ExprKind::Nil`. Parser changes are
limited to recognising `Keyword(Nil)` in `parse_primary` and a new
`strip_span_expr` arm.

### 2. HIR — `ValueKind::Nil`, per-slot kinds, and `infer_kind` rework

`ValueKind` becomes `{ Number, Bool, Nil }`. `LocalInfo` gains a
`kind: ValueKind` field — the single source of truth for the slot
type. `infer_kind` is rewritten to take `(&HirExpr, &[LocalInfo])`
so that `Local(id)` resolves to `locals[id.0].kind` instead of
defaulting to `Number`.

`lower_stmt::Local` computes the initialiser's kind via `infer_kind`
and stores it in the new `LocalInfo`. `lower_stmt::Assign` checks
that the new value's kind matches the existing slot kind and emits
`HirError::TypeMismatch` on mismatch.

### 3. Heterogeneous `==`/`~=` becomes static fold

The 2.2b rule "both sides of `==`/`~=` must share a kind" is replaced
with a Lua-conformant rule:

| `lhs_kind` | `rhs_kind` | `==` lowers to                          | `~=` lowers to                     |
| ---------- | ---------- | --------------------------------------- | ---------------------------------- |
| same       | same       | `HirExprKind::BinOp { op: Eq, ... }`    | `HirExprKind::BinOp { op: Ne, ... }` (codegen handles) |
| different  | (any)      | `HirExprKind::Bool(false)` (folded)     | `HirExprKind::Bool(true)` (folded) |
| both Nil   | both Nil   | `HirExprKind::Bool(true)` (folded)      | `HirExprKind::Bool(false)` (folded) |

The both-Nil case is folded too, because the existing codegen path
for same-kind `==` would emit a comparison on slot values that for
nil are uninteresting (always 0). Static folding keeps codegen
unaware of nil at the comparison level.

### 4. Ordering and arithmetic remain strict

`<`, `<=`, `>`, `>=` continue to require both sides be `Number`. nil
or bool on either side is `HirError::TypeMismatch`. Arithmetic is
unchanged — `Number` only.

### 5. Codegen — per-kind alloca, nil literal, `s_nil` global

`emit_alloca_slot` becomes `emit_alloca_slot_for_kind(ValueKind, ...)`:

- `Number` → `f64` slot (existing behaviour)
- `Bool`   → `i1` slot
- `Nil`    → `i1` slot (the value is never observed; the slot exists
  only so loads/stores have somewhere to land)

`HirExprKind::Nil` lowers to `arith.constant 0 : i1`. This value is
stored to nil-typed slots on init/assign and loaded on access, but
never inspected — print dispatches on the static `ValueKind`, and
heterogeneous `==` folds at HIR time, so the loaded i1 is unused.

A new `s_nil = "nil\0"` string global joins the existing `fmt_str`,
`s_true`, `s_false`. `emit_print_value` gets a `ValueKind::Nil` arm
that emits the same `printf("%s\n", &s_nil)` pattern as the bool path
(no `llvm.select` needed — there is only one nil pointer).

### 6. Type-changing reassignment is rejected

`local x = 1; x = nil` is `HirError::TypeMismatch { op: "=", ... }`.
This diverges from Lua's dynamic typing but is consistent with the
static slot model. Re-evaluation will come when (if ever) we add a
runtime tag layer, which would naturally arrive with tables / GC
in a much later phase.

## Alternatives Considered

- **Tagged-union slot (i8 tag + payload)**. Would unblock dynamic
  typing immediately but requires tag checks at every load, every
  arithmetic op, every comparison, every print. Massive surface
  change for no benefit before tables exist. Rejected.
- **`local x = 1; x = nil` allowed via slot widening.** Either
  requires dynamic tagging (above) or a per-scope retyping rule
  that is hard to reason about with control flow. Rejected.
- **Heterogeneous `==` returns false at runtime via tag compare.**
  Static folding gives the same observable behaviour at zero
  runtime cost. Rejected.
- **`nil` slot represented as `!llvm.ptr` null.** Anticipates the
  GC value layout, but until heap values exist, ptr-nil offers no
  advantage over i1 0 and complicates `Types` plumbing. Rejected.
- **Storing kind in `HirExpr` itself (per-expression type tag).**
  Cleaner uniformity, but doubles HIR memory and requires every
  HirExpr construction site to compute kind. Defer; revisit only
  if `infer_kind` recomputation becomes costly.

## Consequences

- `Keyword` +1 (`Nil`).
- `ExprKind` +1, `HirExprKind` +1, `ValueKind` +1.
- `LocalInfo` gains a `kind` field — every construction site is
  updated.
- `infer_kind` signature changes — every caller updated to pass
  `&chunk.locals` (codegen) or `&self.locals` (HIR).
- `emit_string_global` is called once more for `s_nil`.
- 2.3b can implement `emit_truthiness(value, kind) -> i1` simply
  with a 3-way match on `ValueKind`.

## Out of Scope (still deferred)

- `if cond then ... end`, `if/elseif/else`, `while ... do ... end`
  → Phase 2.3b
- `truthiness(value, kind) -> i1` codegen helper → Phase 2.3b
- `and` / `or` / `not` short-circuit → Phase 2.3c
- Dynamic typing (`local x = 1; x = nil` allowed) → Phase 2.4+
- `nil` as a heap-allocated value, GC layout → much later
- Tables, metatables → Phase 2.5+
