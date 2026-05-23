# 0095. Phase 2.6+-nested-index-assign-widen: Nested Index Target Widening with TAG_TABLE Runtime Narrow

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091 → 0092 → 0093 → 0094 chain closed the method-axis
refinement. The next candidate was **Multi-segment method-def**
(`function a.b.c:m() end`, ADR 0092 carry-over). Codex review (6
視点) returned **Refactor → Go**, but pre-implementation exploration
revealed a deeper prerequisite: `app.utils.field = 10` already
failed today because the nested-IndexAssign target rejects on
`target_kind == Number`:

```
lumelir: hir error: operator '[]=' has incompatible operand types: lhs=table, rhs=number
  | app.utils.field = 10
  | ^
```

The user steered to **non-ad-hoc** via AskUserQuestion. Solving the
chokepoint once unlocks both nested writes AND (in a follow-up ADR
0096, parser-only) multi-segment method-def. This ADR is the
chokepoint fix.

## Non-goals (top-of-ADR)

- **Multi-segment method-def parser support** — future ADR 0096
  reuses this ADR's HIR/codegen path. Parser-only delta.
- **Index read kind narrowing in general value positions** —
  `local t = app.utils` storing as Table-kind local (vs TaggedValue
  via IndexTagged today). Larger ADR; flow-sensitive analysis.
- **TaggedValue with TAG_FUNCTION / TAG_STRING etc. in Index/IndexAssign
  target** — trap at runtime (TAG_TABLE check fails). Surfaces as
  `s_index_target_not_table` trap.
- **Source-order shadowing resolution** — orthogonal ADR 0091+
  carry-over.

## Context

Today's lowering rejects the case at HIR time before codegen sees
anything. The chokepoint is two-fold:
- `lower_expr` `ExprKind::Index` arm (`src/hir/mod.rs:3731+`) —
  requires `target_kind == Table`.
- `lower_stmt_match_arms` `StmtKind::IndexAssign` arm
  (`src/hir/mod.rs:2606+`) — same requirement.

For a nested AST `Index{Index{Ident, Str}, Str}` (e.g.
`app.utils.field`), the inner Index lowers as
`HirExprKind::Index { .. }` whose `infer_kind` returns Number. The
outer check rejects.

Lua runtime semantics: `app.utils` IS a Table — the kind tracking
just can't see through Index reads.

## Reframing

Mirror of ADR 0063's `widen_index_for_local_init` pattern. After this
ADR:

```
HIR (both Index read AND IndexAssign write arms):
  target_hir = lower_expr(target)
  target_hir = widen_index_for_assign_target(target_hir)  // Index → IndexTagged
  target_kind = infer_kind(target_hir)                    // TaggedValue (if widened)
  if target_kind not in {Table, TaggedValue} → TypeMismatch

Codegen (both emit_expr Index arm AND emit_stmt IndexAssign arm):
  if target shape is IndexTagged → emit_resolve_table_target_ptr
       → emit_narrow_indextagged_to_table_ptr (TAG_TABLE check + extract)
  else → existing emit_expr path
```

The widen is **idempotent** on non-Index shapes (single-Ident
targets unchanged). The narrowing helper is shared via
`emit_resolve_table_target_ptr` (one chokepoint reused by Index
read / IndexAssign write / `emit_local_init_tagged` for downstream
`print(t.k)` / `tostring(t.k)` paths).

## Codex 6-視点 reframing

The codex review I ran was for "Multi-segment method-def" (since-
pivoted). Its guidance still applies to the chokepoint fix:

1. **non-ad-hoc / Tidy First**: SINGLE chokepoint fix unlocks
   multiple downstream features (this ADR's nested writes/reads +
   future ADR 0096's multi-segment method-def). Helper extract via
   `emit_resolve_table_target_ptr` keeps the narrowing logic at
   one site.
2. **TDD**: 3 happy + 1 regression-pin Red-per-surface split.
3. **FP**: pure `widen_index_for_assign_target` helper (mirrors ADR
   0063 idempotent shape); effectful codegen branch dispatched via
   the shared resolve helper.
4. **CA**: HIR + codegen change. CA invariant deviation documented;
   bounded to `emit.rs` (175 LOC delta).
5. **Security**: TAG_TABLE runtime check prevents non-Table targets
   from corrupting state. Existing trap-then-yield scf.if pattern
   (ADR 0082/0089) reused.
6. **Documentation**: ADR 0095 captures the pivot rationale,
   chokepoint design, future ADR 0096 pointer.

## New surface

- **`widen_index_for_assign_target`** in `src/hir/mod.rs` (~10 LOC)
  — idempotent helper mirroring `widen_index_for_local_init` shape.
  Rewrites `HirExprKind::Index` → `HirExprKind::IndexTagged` at
  Index target positions.
- **HIR IndexAssign + Index target_kind check loosen**
  (`src/hir/mod.rs:2606+` and `:3731+`) — accept both `Table` AND
  `TaggedValue`.
- **`emit_resolve_table_target_ptr`** in `src/codegen/emit.rs` (~50
  LOC) — dispatch helper: IndexTagged target → narrow; everything
  else → existing `emit_expr`. Used at 3 sites (Index read,
  IndexAssign write, `emit_local_init_tagged` target).
- **`emit_narrow_indextagged_to_table_ptr`** in `src/codegen/emit.rs`
  (~80 LOC) — narrowing chokepoint: alloca tmp tagged slot, run
  `emit_local_init_tagged`, check tag == TAG_TABLE, trap on
  mismatch, extract Table descriptor as `!llvm.ptr` via
  `llvm.inttoptr`.
- **`s_index_target_not_table` trap message global** in `emit.rs`
  initial-globals block — "attempt to index a non-table value\0"
  (Lua spec §3.4.11).

## Reuse

- `widen_index_for_local_init` (`src/hir/mod.rs:129-137`) — template
  shape.
- `emit_local_init_tagged` (existing) — does the IndexTagged-read
  into a tagged slot; reused inside the narrowing helper.
- `emit_alloca_slot_for_kind(TaggedValue)` (`src/codegen/tagged.rs`)
  — tmp slot alloca.
- `emit_addressof` + `emit_exit_with_message` + scf::r#if trap
  pattern (ADR 0082 / 0089) — trap shape.
- `ARRAY_ELEM_OFF_VALUE` (existing) — payload byte offset.
- Existing IndexAssign / Index codegen paths — unchanged after the
  narrow helper returns a Table descriptor.

## Codex critical fixes baked in

- [x] **Single chokepoint** — `emit_resolve_table_target_ptr` shared
  by all 3 consumer sites (Index read, IndexAssign write,
  `emit_local_init_tagged` source).
- [x] **Idempotent widen** — non-Index shapes pass through unchanged.
  Regression-pin (`single_level_assign_unchanged`) catches any
  divergence.
- [x] **Trap reuses existing pattern** — scf.if + `emit_addressof`
  + `emit_exit_with_message`, same as ADR 0082's `s_call_non_function`
  and ADR 0089's `s_arith_on_non_numeric`.
- [x] **Test count split per surface** — happy (write+read) /
  array-key write / write-twice / regression-pin all in separate
  e2e.

## Test count delta

```
Step 0:  1038 → 1039 (3 Red + 1 always-green regression-pin)
Step 1:  1038 → 1039 (widen helper added; unused; tests still Red)
Step 2:  1038 → 1039 (HIR widen + loosen; codegen-stage error now)
Step 3:  1038 → 1042 (codegen narrow + helper extract; 3 Red Green)
Step 4:  1038 → 1042 (clippy + fmt; codegen diff ~175 LOC)
Step 5:  1038 → 1042 (docs only)

Final: 1038 → 1042 green, single atomic commit
  feat(hir,codegen,docs): nested IndexAssign target widening with TAG_TABLE narrow (ADR 0095)
```

## Verification

- `cargo test --no-fail-fast` → **1038 → 1042**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/parser/ src/lexer/ src/cli/ src/pipeline.rs` → **0**
- `git diff --stat src/codegen/` → ~175 LOC (bounded to `emit.rs`:
  one new dispatch helper, one new narrowing helper, three call-site
  swaps, one trap-message global).
- Manual smoke:
  ```bash
  echo 'local app = {}
  app.utils = {}
  app.utils.field = 10
  print(app.utils.field)' > /tmp/n.lua
  cargo run --quiet -- compile /tmp/n.lua && /tmp/n   # → 10
  ```

## Future work

- **ADR 0096 (next)**: Multi-segment method-def parser extension —
  `parse_method_def` accepts `function a.b.c.m()` / `function a.b.c:m()`
  by looping over `.IDENT` segments. HIR `lower_method_def`
  synthesizes nested Index AST for the receiver chain and reuses this
  ADR's `widen_index_for_assign_target` automatically. Parser-only
  delta + tests.
- **General Index-read kind narrowing** — `local t = app.utils`
  storing as Table-kind local (vs TaggedValue today). Requires
  flow-sensitive analysis; bigger ADR.
- **Source-order shadowing resolution** — orthogonal ADR 0091+
  carry-over.

## ADR number / phase tag

ADR 0095 = Nested Index Target Widening with TAG_TABLE Runtime
Narrow. Phase tag: `2.6+-nested-index-assign-widen` under existing
`2.6+ tables / metatables` sub-lane. Lifts the chokepoint that
ADR 0092 carry-over "multi-segment method-def" depends on, plus
unlocks general nested table writes/reads.
