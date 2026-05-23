# 0027. Phase 2.7d: Lexicographic String Comparison

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7b lit up `==` / `~=` for String operands; Phase 2.7c
shipped `tostring` and concat auto-coercion. The remaining gap in
the string comparison surface is the four ordering operators —
`<`, `<=`, `>`, `>=` — which Lua specifies as lexicographic on
strings (per byte, like libc `strcmp`).

This phase widens the rule from "Number-only ordering" to
"Number-Number or String-String ordering", reusing the `strcmp`
extern that Phase 2.7b already declared.

## Decision

### 1. HIR: kind compatibility relaxed for ordering

`lower_expr`'s `BinOp::Lt | Le | Gt | Ge` arm previously required
both operands to be `Number`. Phase 2.7d admits a second
combination — both `String` — while still rejecting any cross-kind
shape:

```rust
let ok = (lk == ValueKind::Number && rk == ValueKind::Number)
    || (lk == ValueKind::String && rk == ValueKind::String);
```

`infer_kind` is unchanged — ordering still produces `Bool`.

### 2. Codegen: reuse the strcmp dispatch

`emit_expr`'s String-comparison shortcut (introduced in Phase 2.7b
for `==` / `~=`) is generalised: the matcher accepts the four
ordering operators in addition to `Eq` / `Ne`. The common helper
is renamed `emit_string_eq → emit_string_cmp` to reflect its
broader role.

The helper computes `cmp = strcmp(a, b)` once, then projects to an
i1 via `arith.cmpi <pred> cmp, 0`:

| HIR `BinOp` | `arith.cmpi` predicate |
|-------------|------------------------|
| `Eq`        | `Eq`                   |
| `Ne`        | `Ne`                   |
| `Lt`        | `Slt` (signed)         |
| `Le`        | `Sle` (signed)         |
| `Gt`        | `Sgt` (signed)         |
| `Ge`        | `Sge` (signed)         |

`strcmp` returns a signed `int`, so the **signed** integer
predicates (`S*`) are required for the ordering cases. The
equality cases work with either sign-aware predicate since they
test against zero.

### 3. No new runtime surface

`strcmp` was already declared in Phase 2.7b
(`emit_string_runtime_decls`). No additional libc decls,
allocator calls, or globals are needed.

## Alternatives Considered

- **Compare via Lua's full collation rules** (locale-aware,
  Unicode-aware). Outside our subset; rejected.
- **Inline strcmp** as a custom MLIR op chain instead of calling
  libc. Avoids the external symbol but adds a non-trivial loop.
  `strcmp` is universally available; the libc call is the right
  default.
- **Allow cross-kind comparison** (`"1" < 2`) by auto-coercing
  one side. Diverges from Lua, which raises a runtime error on
  cross-kind ordering. Rejected.

## Consequences

- HIR: a single 3-line change in the ordering arm of
  `lower_expr`'s `BinOp` matcher.
- Codegen: `emit_string_eq` renamed to `emit_string_cmp`; the
  predicate map grows four entries; the call-site matcher in
  `emit_expr` widens to include the four ordering operators.
- Nine integration tests in `phase2_7d_string_compare.rs` cover
  the four operators across true/false outcomes, the prefix case
  (`"ab" < "abc"`), use as an `if` condition, the cross-kind
  reject, and a Number-Number regression.

## Out of Scope

- **Locale-aware / Unicode collation**.
- **Cross-kind auto-coercion** for ordering operators.
- **`<=` / `>=` semantics for tables and userdata** — needs the
  metatable system (Phase 2.6+).
