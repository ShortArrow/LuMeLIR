# 0188. Non-Local TaggedValue Source — Residual Dispatcher Sites

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-14
- **Deciders:** ShortArrow

## Context

[ADR 0179](0179-non-local-tagged-source-materialisation.md) introduced `materialize_tagged_source_if_needed` and applied it at 3 RawGet / RawSet source positions. [ADR 0187](0187-indexassign-value-side-tagged-source.md) extended its reach to the IndexAssign value position. Both retrospectives predicted more sites lurking — the ADR 0179 §References §Codex retrospective and the sweep-0166-0177 §Horizontal duplication note both flag that opening one site at a time re-introduces the ad-hoc duplication the chokepoint just collapsed.

A systematic audit (commit-less probes, removed after) confirms **4 remaining gaps** where a non-Local TaggedValue source (Call-return) still trips the codegen `UnsupportedExpr` rejection:

| Gap | Site | Failing diagnostic |
|---|---|---|
| G1 | `rawset(t, k, pick(...))` value position | `rawset TaggedValue value requires Local source (ADR 0176 scope)` |
| G2 | `t[pick(...)]` Index read key | `IndexTagged TaggedValue key requires Local source (ADR 0177 scope)` |
| G3 | `a.b[pick(...)]` IndexTagged read key | same as G2 (shared codegen dispatcher) |
| G4 | `t[pick(...)] = v` IndexAssign key | `TaggedValue-key IndexAssign requires Local key (ADR 0084 scope)` |

Each is structurally identical to the gaps ADRs 0179 and 0187 closed: HIR pre-binds the non-Local source into a synth Local via the existing chokepoint; codegen stays oblivious. This ADR collapses all 4 in one sweep.

## Scope (literal)

- ✅ **G1** — Loosen the `should_materialise_arg2` gate at `src/hir/mod.rs:5041-5053`. ADR 0179 narrowed it to `key is Local(TaggedValue)` because that was the case the original ADR tested; in fact codegen at `src/codegen/emit.rs:11021-11030` rejects ANY non-Local TaggedValue value regardless of key kind. Materialise arg[2] whenever it is TaggedValue.
- ✅ **G2 + G3** — Route `key_hir` through `materialize_tagged_source_if_needed` in the `ExprKind::Index` lowering arm at `src/hir/mod.rs:4419-4428`, after the existing `let key_hir = self.lower_expr(key)?;`. Index → IndexTagged target widening (ADR 0095) happens earlier; the key materialisation feeds both Index and IndexTagged downstream dispatchers via the same shared codegen path.
- ✅ **G4** — Route `key_hir` through the same helper in the `StmtKind::IndexAssign` lowering arm at `src/hir/mod.rs:3389-3398`, after the existing `let key_hir = self.lower_expr(key)?;` (next to ADR 0187's value-side materialisation that landed in the previous ADR).
- ✅ Codegen — **no change**. The Local-only checks at `emit.rs:11026`, `5459`, and `4029` become defensive `unreachable!`-style guards; the comment text could be updated to reflect this but is not required.
- ❌ Any Index / IndexAssign / IndexTagged dispatcher site already covered by ADR 0179 / ADR 0187 (RawGet/RawSet keys, IndexAssign value).
- ❌ Methodology-level changes (e.g. introducing a `materialize_all_tagged_args` macro). Out of scope; each site is a 1-line addition and the chokepoint is the helper.
- ❌ Runtime semantic correctness questions (Nil-tag soft-delete, etc.) — orthogonal, documented in ADR 0187 §Future work.

## Decision

### `src/hir/mod.rs`

**G1 — loosen `lower_builtin_call` arg[2] gate** (line ~5041-5053):

```rust
// ADR 0188 — loosen ADR 0179's arg[2] gate. Codegen rejects ANY
// non-Local TaggedValue value source for RawSet (`emit.rs:11021`),
// not only when the key is Local(TaggedValue). Materialise arg[2]
// whenever it is TaggedValue, regardless of key kind.
let should_materialise = i == 1
    || (i == 2
        && matches!(builtin, Builtin::RawSet)
        && matches!(
            infer_kind(&arg, &self.locals, &self.functions),
            ValueKind::TaggedValue
        ));
```

**G2 + G3 — Index read key materialisation** (line ~4428):

```rust
let key_hir = self.lower_expr(key)?;
// ADR 0188 — IndexTagged dispatcher (`emit.rs:5459`) requires a
// Local source for a TaggedValue key. Pre-bind non-Local
// (Call-return) sources via the chokepoint helper. Idempotent
// for Local and non-Tagged sources.
let key_hir = self.materialize_tagged_source_if_needed(key_hir, key.span)?;
```

**G4 — IndexAssign key materialisation** (line ~3397, next to the existing ADR 0187 value-side materialisation):

```rust
let key_hir = self.lower_expr(key)?;
// ADR 0188 — codegen TaggedValue-key IndexAssign dispatcher
// (`emit.rs:4029`) requires a Local source. Same chokepoint
// reuse as ADR 0187's value-side fix.
let key_hir = self.materialize_tagged_source_if_needed(key_hir, key.span)?;
let value_hir = self.lower_expr(value)?;
let value_hir = self.materialize_tagged_source_if_needed(value_hir, value.span)?;
```

### Tests

`tests/phase2_6plus_non_local_tagged_residuals.rs` (NEW, 4 e2e):
1. **G1**: `rawset(t, "k", pick(true))` with non-Local Call-return value.
2. **G2**: `t[pick(other)]` Index read with non-Local Call-return key.
3. **G3**: `a.b[pick(other)]` IndexTagged read with non-Local Call-return key.
4. **G4**: `t[pick(other)] = 99` IndexAssign with non-Local Call-return key.

All Red Day 0 (probe confirmed each fails today at the noted diagnostic).

## Alternatives considered

- **Open the codegen Local-only checks to handle non-Local sources directly.** Rejected — `emit_expr` on a TaggedValue Local traps on non-Number tag (per existing comments at `emit.rs:4049`, `11019`). The HIR materialisation is the principled fix, established by ADR 0179.
- **One ADR per site (4 sequential micro-ADRs).** Rejected — ADR 0179 retrospective specifically called out the horizontal-duplication anti-pattern; bundling matches the existing pattern and shares one test file.
- **Make the chokepoint helper traverse the entire HIR tree post-lowering.** Rejected — would require a separate pass and lose the spatial locality with each call site. The per-arm explicit call is small (1 line) and discoverable.
- **Extend codegen IndexAssign target-side check (`a.b.c[pick(...)] = v`) at the same time.** Already covered by G4 — the IndexAssign key materialisation runs regardless of whether the target is Index or IndexTagged (ADR 0095 widens earlier in the same arm).

## Consequences

**Positive**
- 4 idiomatic Lua patterns now compile: rawset call-return value, indexed read with call-return key, indexed write with call-return key, nested indexed read with call-return key.
- The "Local-only" scope-ceiling clauses from ADRs 0084 / 0176 / 0177 retire for the audited positions. The codegen `UnsupportedExpr` paths become defensive guards behind a HIR invariant.
- One more horizontal duplication collapsed; the predicted next-after-0179 sweep is now closed.

**Negative**
- HIR adds 3 single-line materialisation calls. Surface increase minimal.
- ADR 0179's `should_materialise_arg2` gate becomes simpler — one branch removed.

**Locked in until superseded**
- `materialize_tagged_source_if_needed` is the SoT for non-Local TaggedValue → synth Local conversion. Any newly added TaggedValue-consuming dispatcher routes through it.

## Documentation updates

- [x] §8 — adds 0188.
- [x] ADR 0179 §Scope — arg[2] gate text updated by reference here; ADR 0179 itself unchanged (the gate broadening is documented as ADR 0188's contribution).
- [x] ADR 0187 §Future work — "Source-position TaggedValue normalisation across any remaining dispatcher" bullet → RESOLVED.

## Test count delta

```
Step 0: 1423 (after 0be360c)
C1 (doc): 1423 → 1423
C2 (4 Red Day 0 e2e): 1423 → 1423 (Red)
C3 (HIR impl): 1423 → 1427 (Green)
```

## Critical files

- `src/hir/mod.rs`:
  - Loosen `should_materialise_arg2` gate in `lower_builtin_call` (~line 5053).
  - Add `materialize_tagged_source_if_needed` call after `let key_hir = ...` in `ExprKind::Index` lowering (~line 4428).
  - Add `materialize_tagged_source_if_needed` call after `let key_hir = ...` in `StmtKind::IndexAssign` lowering (~line 3397).
- `tests/phase2_6plus_non_local_tagged_residuals.rs` (NEW) — 4 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Materialisation fires on a Local TaggedValue source, growing the IR unnecessarily | Helper is idempotent on `HirExprKind::Local(_)`. |
| Loosening the RawSet arg[2] gate triggers materialisation when codegen would have already accepted | The previous gate was `key is Local(TaggedValue)` AND `value is non-Local TaggedValue`; codegen's substitution path runs regardless of whether materialisation fires, so the broader gate is strictly safer. |
| Index read key materialisation runs unnecessarily when key kind is Number / String / Bool | Helper kind check short-circuits non-TaggedValue kinds. No-op for Number / String / Bool / Function / Table. |
| `materialize_tagged_source_if_needed` interacts badly with the ADR 0095 Index→IndexTagged target widening already in the same arm | The widening operates on `target_hir`, the materialisation operates on `key_hir` — orthogonal. |
| 5th gap exists that this audit missed | Audit covered every grep match for "Local" / "TaggedValue" scope-ceiling string. A new gap requires either a new dispatcher or a comment shape this audit didn't catch; the ADR 0179 retrospective predicts the pattern stops here for now. |

## Future work

- **Codegen Local-only check rewording**: the `UnsupportedExpr` text could be updated to "internal: HIR materialisation invariant violated" since the messages are now structurally unreachable; defer until a future Tidy First sweep groups all 4 sites.
- ~~**Audit Method`Call` arg positions**~~ — RESOLVED by post-impl audit (2026-06-15, commit-less probes removed after). Surveyed `print` / `tostring` / `type` / `string.format` / `MethodCall` receiver / `assert` / `pairs` / `ipairs` / `next` / `setmetatable` arg positions with non-Local TaggedValue Call-return sources. All either work today (`print` / `tostring` / `type`) or are cleanly rejected at HIR with documented diagnostics keyed to other ADRs (ADR 0152 for `string.format`, ADR 0092 for complex receivers, kind-mismatch for `assert`). No codegen `UnsupportedExpr` / `unreachable!` panics surfaced in the audited surface. The non-Local TaggedValue source sweep is closed for the dispatchers currently exposed to user-level Lua.
- **Cross-procedure non-Local TaggedValue refinement** — when a callee returns a known-narrow TaggedValue (e.g. always-String), the materialisation could be elided. Speculative.

## References

- [ADR 0084](0084-phase2-8e-iter-tk.md) — first Local-only TaggedValue-key scope ceiling.
- [ADR 0176](0176-rawset-tagged-value-local-source.md) — RawSet value Local-only scope.
- [ADR 0177](0177-index-tagged-key.md) — IndexTagged Local-only key scope.
- [ADR 0179](0179-non-local-tagged-source-materialisation.md) — `materialize_tagged_source_if_needed` chokepoint; this ADR extends the same helper.
- [ADR 0187](0187-indexassign-value-side-tagged-source.md) — IndexAssign value-side materialisation; direct precedent for the per-arm reuse pattern.
- [`docs/notes/sweep-0166-0177-retrospective.md`](../notes/sweep-0166-0177-retrospective.md) — Horizontal-duplication insight predicting more sites lurking.
