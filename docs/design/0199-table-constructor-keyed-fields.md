# 0199. Table Constructor — Bracket-Key and Named-Key Fields

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

[Lua 5.4 §3.4.9](https://www.lua.org/manual/5.4/manual.html#3.4.9) defines the table constructor with three field forms:

```
field ::= '[' exp ']' '=' exp |
          Name '=' exp |
          exp
```

LuMeLIR currently supports only the positional form (`exp`). Bucket E §E6 ([leftover roadmap](../notes/leftover-roadmap.md)) flagged the bracket-key form (`{[k]=v}`); the named-key form (`{name=v}`) is the same parser change with the same HIR lowering, so they bundle into one ADR.

## Scope (literal)

- ✅ Parser accepts `[exp] = exp`, `Name = exp`, and `exp` field forms (Lua 5.4 §3.4.9 exhaustive set).
- ✅ AST `ExprKind::Table(Vec<TableField>)` where `TableField = Positional(Expr) | Keyed { key: Expr, value: Expr }`.
- ✅ HIR mirrors the AST shape. `lower_expr` for `Table` materializes a synth Local, emits IndexAssign pre-statements for keyed fields, and returns `Local(synth_id)`.
- ✅ Mixed positional + keyed in one constructor (`{1, name="x", [3]=true}`) works per Lua spec.
- ✅ Trailing comma + semicolon separators per Lua spec (current behaviour preserved for positional; extends to keyed forms uniformly).
- ❌ Hash-key kind other than String / Number / Bool / Function / Table (i.e. `[nil]=v` or `[NaN]=v`). Existing HIR `is_hash_key_eligible` gate rejects; ADR 0199 inherits.
- ❌ Numeric/computation-based positional index reassignment. Positional indices restart at 1 within a constructor; keyed-bracket with integer key works normally per Lua spec.
- ❌ Codegen-level restructuring. Keyed fields desugar via existing `IndexAssign` paths; no new emit arm.

## Decision

### AST (`src/parser/ast.rs`)

```rust
pub enum TableField {
    Positional(Expr),
    Keyed { key: Expr, value: Expr },
}

pub enum ExprKind {
    // ...existing variants...
    Table(Vec<TableField>),  // was: Vec<Expr>
}
```

### Parser (`src/parser/mod.rs::parse_primary` LBrace arm)

For each field within `{...}`:

1. Peek next token.
2. If `LBracket` (`[`): consume `[`, parse expr (key), expect `]`, expect `=`, parse expr (value) → `Keyed { key, value }`.
3. If `Ident(name)` AND the token after is `=` (single-token lookahead): consume Ident, consume `=`, parse expr → `Keyed { key: Str(name), value }`. Synthesize a string-literal expr for the key with the same span.
4. Otherwise: parse expr → `Positional(expr)`.

Field separator stays `,` or `;` (Lua spec). Trailing separator allowed.

### HIR (`src/hir/ir.rs`)

```rust
pub enum TableField {
    Positional(HirExpr),
    Keyed { key: HirExpr, value: HirExpr },
}

pub enum HirExprKind {
    // ...
    Table(Vec<TableField>),  // shape unchanged for positional-only consumers
}
```

### HIR lowering (`src/hir/mod.rs::lower_expr` Table arm)

If the table contains **only** Positional fields: existing path; emit `HirExprKind::Table(...)` with all entries as `TableField::Positional(...)`.

If any Keyed fields present:

1. Materialize a synth Local for the empty table (positional-only `Table` HirExpr with the positional subset preserved as field initialisers).
2. For each `Keyed { key, value }`: push an `IndexAssign` HIR statement into `pending_pre_stmts` (same chokepoint ADR 0179 uses): target = `Local(synth)`, key = lowered key, value = lowered value.
3. Return `Local(synth)` as the Table expression value.

Positional and keyed entries in the same constructor: positional fields go into the synth Local's initial Table (numeric indices 1..n per Lua spec); keyed fields go into the IndexAssign pre-statements that run immediately after the synth Local is initialised.

### Codegen — no change

The Table emit arm sees only `HirExprKind::Table` with `Vec<TableField::Positional>` (post-lowering). IndexAssign pre-statements run through the existing path. Net codegen surface: 0 LOC change.

### Tests

`tests/phase4_table_keyed_fields.rs` (NEW, 4 e2e):

1. Bracket-key Number: `local t = {[1]=99}; print(t[1])` → `"99"`.
2. Named-key String-sugar: `local t = {name="x"}; print(t.name)` → `"x"`.
3. Mixed: `local t = {10, name="y", [3]=true}; print(t[1], t.name, t[3])` → `"10\ty\ttrue"`.
4. Trailing-comma + keyed: `local t = {a=1,}; print(t.a)` → `"1"`.

## Alternatives considered

- **Parser-only desugar to a synth-statement sequence.** Rejected — parser context is expression-only; cannot emit statements. HIR is the right layer.
- **AST `Vec<(Option<Expr>, Expr)>` where None means positional.** Rejected — less self-documenting than a named-variant enum.
- **Defer named-key (`{name=v}`) to a separate ADR.** Rejected — same parser arm, same HIR lowering. Bundling halves the surface.

## Consequences

**Positive**
- `{[k]=v, name=v}` idiomatic Lua works.
- Bucket E §E6 RESOLVED.
- Codegen surface unchanged — existing IndexAssign / Table paths handle everything.

**Negative**
- AST + HIR `Table` variant signature changed; downstream `match` consumers in `infer_kind`, `lower_stmt`, and codegen need a small adaptation (positional vs keyed pattern destructuring).

**Locked in until superseded**
- `TableField` enum naming. Future fields (e.g. spread `...`) would extend the enum.

## Documentation updates

- [x] §8 — adds 0199.
- [x] Bucket E §E6 marked RESOLVED in `bucket-e-probe-results.md`.

## Test count delta

```
Step 0: 1438 (after f1f6af5)
C1 (doc): 1438 → 1438
C2 (4 Red Day 0 e2e): 1438 → 1438 (Red — parser rejects bracket and named)
C3 (impl): 1438 → 1442 (Green)
```

## Critical files

- `docs/design/0199-table-constructor-keyed-fields.md` (this doc).
- `docs/design/README.md` index entry.
- `src/parser/ast.rs` — `TableField` enum + Table variant signature.
- `src/parser/mod.rs::parse_primary` LBrace arm — bracket-key and named-key detection.
- `src/hir/ir.rs` — `TableField` enum.
- `src/hir/mod.rs::lower_expr` Table arm — keyed desugar.
- `src/codegen/emit.rs` — Table arm signature adaptation (positional-only consumption).
- `tests/phase4_table_keyed_fields.rs` (NEW) — 4 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Single-token lookahead for `Name =` conflicts with `Name` as positional ident-expression | Disambiguated by the `=` follow-set; if next token is not `=`, fall back to positional. Same disambiguation pattern Lua reference parsers use. |
| Mixed positional + keyed ordering surprises | Positional indices restart at 1 within constructor regardless of keyed interleaving (Lua spec). Test 3 pins this. |
| HIR pending_pre_stmts in Table-expr context not exercised today | ADR 0179's `materialize_tagged_source_if_needed` already runs through the same chokepoint inside `lower_builtin_call`, which IS expr context. Pattern validated. |
| Codegen Table arm assumes `Vec<HirExpr>` | Adapter wraps fields → positional vec; reverts to existing path. Surface change is small. |

## Future work

- ADR (TBD) — Spread expression `{...args}` within table constructors (Lua 5.4 allows the last positional field to be a vararg). Not part of bucket E.
- ADR (TBD) — Vararg `...` overall (bucket E §E7).

## References

- [Lua 5.4 §3.4.9 Table Constructors](https://www.lua.org/manual/5.4/manual.html#3.4.9)
- [ADR 0053](0053-phase2-6a-min-table.md) — minimal empty table.
- [ADR 0054](0054-phase2-6a-arr.md) — positional array form.
- [Bucket E probe results §E6](../notes/bucket-e-probe-results.md)
