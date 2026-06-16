# 0209. Integer AST + HIR Variant (Phase B Opt-In)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Goal **M1** (per `docs/notes/roadmap-revision-2026-06-16.md`) = Integer/Float subtype core. First implementation step after ADR 0197 (Phase A lexer).

ADR 0196 §Migration outlines three phases:

- **Phase A (DONE in ADR 0197)** — additive lexer-only distinction at the token layer.
- **Phase B (THIS ADR begins)** — opt-in: integer literals route through dedicated AST + HIR variants; codegen demotes via `sitofp` to preserve all 125 `ValueKind::Number` consumer sites untouched.
- **Phase C (future)** — strict: codegen lifts the demotion at integer-aware operations.

This ADR delivers the minimum Phase B step: AST `ExprKind::Integer(i64)` + HIR `HirExprKind::Integer(i64)`. `infer_kind` continues to return `ValueKind::Number` for `HirExprKind::Integer`, leaving the 125 downstream consumers (arith / comparison / metamethod / printf-format / etc.) untouched.

## Scope (literal)

- ✅ AST `ExprKind::Integer(i64)` (new variant, alongside `Number(f64)`).
- ✅ Parser: `TokenKind::Integer` → `ExprKind::Integer` (was demoted via `as f64` to `Number` in ADR 0197 parser arm).
- ✅ HIR `HirExprKind::Integer(i64)` (new variant).
- ✅ HIR `lower_expr` for `ExprKind::Integer` → `HirExprKind::Integer`.
- ✅ HIR `infer_kind` for `HirExprKind::Integer` returns `ValueKind::Number` (Phase B silent demotion at the kind layer).
- ✅ Codegen `HirExprKind::Integer(i)`: emit `arith.constant i : i64` then `arith.sitofp i64 → f64`, demoting at the leaf so downstream f64 paths receive an indistinguishable value.
- ✅ `check_method_receiver_shape`, `strip_span_expr` updated to accept `Integer` in the allowed-shape sets.
- ✅ Existing parser / HIR / codegen tests migrated: assertions that previously expected `Number(N.0)` for integer-syntax sources now expect `Integer(N)` (1457 tests stay green).
- ❌ `ValueKind::Integer` — deferred. Adding it requires touching the 125 `ValueKind::Number` consumer sites and would inflate this ADR's surface beyond a Phase B step. Future ADR (0210 candidate) introduces it.
- ❌ Integer-aware codegen ops (i64 add/sub/cmp). Phase C territory.
- ❌ `tostring(i)` subtype-distinguished formatting (`"1"` vs `"1.0"`). Phase C territory.
- ❌ Mixed-subtype arithmetic rules per Lua §3.4.1. Phase C territory.

## Decision

### `src/parser/ast.rs`

```rust
pub enum ExprKind {
    Number(f64),
    /// ADR 0209 — Phase B opt-in integer literal.
    Integer(i64),
    // ...existing variants...
}
```

### `src/parser/mod.rs::parse_primary`

```rust
TokenKind::Integer(value) => {
    self.bump();
    Ok(Expr::new(ExprKind::Integer(value), tok.span))
}
```

(was: `Ok(Expr::new(ExprKind::Number(value as f64), tok.span))`)

### `src/hir/ir.rs::HirExprKind`

```rust
pub enum HirExprKind {
    Number(f64),
    /// ADR 0209 — Phase B opt-in. infer_kind returns Number to
    /// keep the 125-site consumer surface untouched; codegen
    /// demotes via sitofp.
    Integer(i64),
    // ...existing variants...
}
```

### `src/hir/mod.rs::lower_expr`

```rust
ExprKind::Integer(i) => HirExprKind::Integer(*i),
```

### `src/hir/mod.rs::infer_kind`

```rust
HirExprKind::Integer(_) => ValueKind::Number,
```

(Phase B silent demotion at the kind layer.)

### `src/codegen/emit.rs::emit_expr`

```rust
HirExprKind::Integer(i) => {
    let i64_const = block.append_operation(arith::constant(...i64...)).result(0).unwrap().into();
    let as_f64 = block.append_operation(arith::sitofp(i64_const, types.f64, loc)).result(0).unwrap().into();
    Ok(as_f64)
}
```

(Emits `arith.constant N : i64` then `arith.sitofp N : i64 to f64`.)

### Tests

- Existing parser tests using whole-number sources migrated from `number(N.0)` helper to new `integer(N)` helper (33 sites bulk-replaced).
- HIR test `lower_print_constant_produces_print_call` asserts `HirExprKind::Integer(42)`.
- HIR test `lower_assign_to_existing_local_resolves` asserts `HirExprKind::Integer(2)`.
- HIR test `lower_for_numeric_default_step_inserts_constant_one` asserts source integers as `Integer`, synth step as `Number(1.0)` (codebase-injected, not source).
- Codegen test `emit_number_constant_produces_arith_constant` asserts `arith.constant 42 : i64` + `arith.sitofp` (was `arith.constant 4.200000e+01`).

No new e2e tests — Phase B silent demotion preserves all 1431 existing e2e outputs. Lib test count rises 1453 → 1457 due to migrated assertions.

## Alternatives considered

- **Skip AST variant; demote at parser like ADR 0197.** Rejected — defeats the purpose of Phase B (preserving integer info into HIR for future Phase C uplift).
- **Add `ValueKind::Integer` simultaneously.** Rejected — touches the 125 `ValueKind::Number` consumer surface, exceeds a Phase B step. Future ADR.
- **Codegen emits f64 constant directly without sitofp.** Rejected — silent demotion via sitofp preserves the i64 origin in the LLVM IR for debugger / future Phase C optimisation passes; an f64 constant erases it.
- **Bundle Phase C integer-aware ops.** Rejected — Phase B's value is incremental boundary; bundling Phase C makes regression risk too high in one ADR.

## Consequences

**Positive**
- Integer info preserved through AST + HIR.
- All 1431 existing e2e tests stay green (silent demotion at kind + codegen).
- Future Phase C ADRs can pattern-match `HirExprKind::Integer` to lift the demotion site-by-site.

**Negative**
- Parser test corpus required 33-site mechanical migration (one helper rename).
- 4 HIR/codegen unit tests required assertion update.
- AST + HIR enums grow by one variant each (well-confined surface).

**Locked in until superseded**
- `HirExprKind::Integer` carries i64; codegen demotion via sitofp is the v1 lowering.
- `infer_kind` returns `Number` for Integer until ADR 0210+ introduces `ValueKind::Integer`.

## Documentation updates

- [x] §8 — adds 0209.
- [x] `docs/notes/roadmap-revision-2026-06-16.md` — M1 in progress (first sub-ADR landed).

## Test count delta

```
Step 0: 1457 (after 4b595da)
C3 (impl + test migration): 1457 → 1457 (unit-test count unchanged
                                          after migration; existing
                                          1431 e2e + lib tests all
                                          remain green)
```

Actual final: 1457 pass / 0 fail.

## Critical files

- `docs/design/0209-integer-ast-hir-variant.md` (this doc).
- `docs/design/README.md` index entry.
- `src/parser/ast.rs` — `ExprKind::Integer`.
- `src/parser/mod.rs` — parser arm + `strip_span_expr` + new `integer()` test helper + 33-site bulk migration.
- `src/hir/ir.rs` — `HirExprKind::Integer`.
- `src/hir/mod.rs` — `lower_expr` arm, `infer_kind` arm, `check_method_receiver_shape` arm, HIR test assertions.
- `src/codegen/emit.rs` — `HirExprKind::Integer` emit arm + codegen-test assertion update.

## Future work

- ADR 0210 — Introduce `ValueKind::Integer` and update 125-site consumer surface. Keeps Phase B silent demotion at codegen until per-op lifts arrive.
- ADR 0211+ — Per-op Phase C uplifts: integer arithmetic, integer comparison, integer-typed `tostring`, mixed-subtype rules per Lua §3.4.1, bitwise integer requirement.

## References

- [ADR 0196](0196-integer-float-subtype-design.md) — Integer/Float subtype design entry.
- [ADR 0197](0197-integer-literal-token-additive.md) — Phase A lexer addition.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone definition.
- [Lua 5.4 §3.4.1 Arithmetic Operators](https://www.lua.org/manual/5.4/manual.html#3.4.1) — subtype rules.
