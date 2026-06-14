# Sweep 0182-0188 Retrospective (Codex 6 Lens)

- **Date:** 2026-06-15
- **Scope:** ADRs 0182 → 0188 (7 ADRs + 5 ancillary commits) — Phase 3 GC infrastructure + TaggedValue source-position normalisation closure + Phase 2 formal close
- **Sweep delta:** tests 1407 → 1427 (+20), `src/codegen/emit.rs` ~ +560 LOC, `src/codegen/primitive.rs` ~ +160 LOC, `src/codegen/tagged.rs` ~ +60 LOC, `src/hir/mod.rs` ~ +60 LOC net
- **Authoring cadence:** doc (C1) → Red Day 0 (C2) → impl + push (C3), with mid-sweep Codex 6視点 pre-flight review for the GC ADRs

## What landed

| ADR | Theme | Tidy First / Feature | Key shape |
|---|---|---|---|
| 0182 | Param-inference kind-parameterised helpers | Tidy First | `mark_ident_as(expr, kinds, kind)` + `is_body_decisive_kind` predicate; collapses 0180/0181 duplication |
| 0183 | MethodCall String receiver namespace dispatch | Feature | Consumes 0182; `s:upper()` short-circuits to `Builtin::string_from_method` when receiver is `ValueKind::String` |
| 0184 | GC type metadata + size guard | Tidy First prep | `gc_type_meta(type_tag)` decision table in `tagged.rs` + ≥ 4 GiB payload guard in `emit_gc_alloc`; absorbs R2/R3 from pre-flight memo |
| 0185 | GC v1 safety-mode mark + sweep walk | Feature (structural) | `emit_gc_mark_inline` + `emit_gc_sweep_inline` per ADRs 0159/0161; v1 pre-paints BLACK so freeing is deferred to ADR 0189+ |
| 0186 | GC auto-trigger threshold + llvm.func factoring | Feature + Tidy First | 1 MiB threshold per ADR 0162; ADR 0185 inline walks promoted to `llvm.func @gc_mark` / `@gc_sweep` when second caller arrived |
| 0187 | IndexAssign value-side TaggedValue | Feature | Closes static-key `unreachable!` at `emit.rs:3980`; HIR materialisation via ADR 0179 helper, codegen TaggedValue arm symmetric to ADR 0138-M |
| 0188 | Residual non-Local TaggedValue source sites | Feature | 4 audit-found gaps closed via same chokepoint (RawSet value, Index/IndexTagged key, IndexAssign key) |

Ancillary commits worth noting:

- **`f30a7c1`** — ADR 0103 `tests/phase2_stdlib_string.rs:19` deferred annotation lifted (`s:len()` no longer needs metatable per ADR 0183) + String-literal Local regression pin
- **`60932bf`** — ADR 0133 §Decision table updated to mark Phase 2 closed (Number-key `__newindex` matrix had already landed across ADRs 0168-0171, but the table was stale)
- **`77638e0`** — Codex 6視点 pre-flight review memo (`docs/notes/gc-0159-0162-preflight-review.md`) before starting GC implementation; surfaced R1 (worklist capacity) / R2 (per-type dispatch chokepoint) / R3 (size truncation)
- **`0be360c`** — Post-impl audit pin: IndexTagged target uses same `emit_resolve_table_target_ptr` normalisation so ADR 0187 already covers nested write targets at any depth
- **`f6962b2`** — MethodCall arg-position audit follow-up closing ADR 0188 §Future work
- **`88491b6`** — Tidy First refactor: 5 codegen `UnsupportedExpr("requires Local source")` sites promoted to `unreachable!()` with ADR citations; HIR invariant is now structurally load-bearing

Closed deferral / future-work rows:

- ADR 0103 "`s:len()` method syntax (needs metatable)" — RESOLVED by ADR 0183 (receiver-kind dispatch, no metatable required for String primitive)
- ADR 0133 "Number-key (array) `__newindex` remains a separate ADR" — RESOLVED (already landed across 0168/0169/0170/0171; doc update only)
- ADR 0157 R3 "u32 size header truncation" — RESOLVED in 0184
- ADR 0159 R2 "per-type dispatch chokepoint" — partially RESOLVED in 0184 (metadata bool); per-type walk strategy still ADR 0189+ scope
- ADR 0181 "MethodCall form `s:upper()` String inference" — RESOLVED by ADR 0183
- ADR 0187 "IndexTagged target-side TaggedValue value" — RESOLVED by audit pin
- ADR 0188 "MethodCall arg-position audit" — RESOLVED by post-impl probe (no gap found)
- ADR 0188 "Codegen Local-only check rewording" — RESOLVED by `88491b6` (promoted to `unreachable!`)

## Codex 6 lens

### #1 anti-ad-hoc / Tidy First / Robust — **strong**
- **Tidy First applied at the right rule-of-three points**: ADR 0182 collapsed 0180/0181 marker duplication with only 2 consumers (precedent for "extract before the third consumer"). ADR 0186 promoted ADR 0185's inline walks to `llvm.func` when the second caller (auto-trigger) materialised. Both followed the doctrine without over-extracting.
- **Robust axis honored** via the pre-flight review process: ADR 0184 absorbed R3 (size truncation guard) and the R2 partial chokepoint before ADR 0185 mark phase could surface them at runtime.
- **One mid-sweep Codex 6視点 retrospective on the GC arc** before starting implementation (`docs/notes/gc-0159-0162-preflight-review.md`) — same lens as this retro, applied prospectively. Cost: 1 commit. Saved: at least 1-2 implementation surprises.

### #2 TDD — **clean except for structural ADRs**
Every code-bearing ADR had Red Day 0 → impl → Green. The GC ADRs (0184/0185/0186) honored the cycle even when v1 safety-mode meant tests were regression pins rather than feature flips — the contracts were still pinned. ADR 0184 + the `88491b6` refactor were behaviour-preserving (no new tests, existing corpus pins).

### #3 FP / responsibility separation — **clean**
HIR materialisation chokepoint (`materialize_tagged_source_if_needed`) extended in ADRs 0187 + 0188 without touching codegen contract; codegen Local-only checks became structural assertions instead of runtime errors. The GC walks live as block-emit functions in `emit.rs`, called either inline or from `llvm.func` body builders — same pure-builder shape used by ADR 0157.

### #4 Clean Architecture / dependency direction — **clean**
`tagged.rs` (constants + small metadata) ← `primitive.rs` (allocator) ← `emit.rs` (orchestration / walks / `func` registration). The GC layering matches the pre-flight review's prediction; no circular dependency emerged.

### #5 Given/When/Then state diff — **clean**
Each ADR has explicit scope literal (✅/❌). The "what runs through the new path" answer is unambiguous in every doc. ADR 0185 §v1 safety mode pin-points the observable-vs-structural distinction — readers should never wonder whether `collectgarbage()` frees bytes in v1.

### #6 Naming / docstring — **clean**
`gc_type_meta`, `emit_gc_mark_inline`, `materialize_tagged_source_if_needed` — every introduced symbol carries intent in the name. ADR-citation comments in code (`ADR 0184 — ...`) survived the Tidy First refactor in `88491b6` because they cite the introducing ADR, not the obsolete `UnsupportedExpr` message text.

## Horizontal-duplication closure (TaggedValue source positions)

The prior retrospective (sweep 0166-0177) §Horizontal duplication called out the "Local-only" scope-ceiling clause as the pattern stop-loss to look for. ADR 0179 closed 3 sites (RawGet/RawSet args). The unfinished work was tracked in the retrospective's §"Next chokepoint candidates".

This sweep closed the rest:

- **ADR 0187** — IndexAssign value-side (originally Day-0 candidate for 0181 but pivoted to param inference).
- **ADR 0188** — 4 residual sites (RawSet arg[2] gate broadening, Index/IndexTagged read key, IndexAssign key) found via systematic audit.
- **Post-0188 MethodCall audit** (`f6962b2`) — confirmed no further user-reachable dispatcher with a non-Local TaggedValue source gap.
- **Post-0188 Tidy First** (`88491b6`) — 5 codegen `UnsupportedExpr` sites promoted to `unreachable!()` with ADR-citation messages. The HIR invariant is now structurally load-bearing: a regression in `materialize_tagged_source_if_needed` surfaces as a clean panic, not a deferred `Result` propagated upward.

**Net result**: every audit-cover dispatcher consuming TaggedValue now routes through one HIR helper. The "Local-only" scope clause is retired across ADRs 0084 / 0138-M / 0176 / 0177 for the audited surface.

## Lessons (durable, additive to prior retro)

7. **Codex 6視点 pre-flight review before a multi-ADR arc pays off.** The GC pre-flight memo (`77638e0`) named the three robustness gaps (R1/R2/R3) and pinned the implementation ADR boundary. ADR 0184 absorbed two; R1 stays with ADR 0189+ where mark loop complexity makes it intrinsic. Without the memo, R3 (size truncation) would likely have shipped as a silent bug in ADR 0185.
8. **"Extract before the third consumer" generalises beyond markers.** ADR 0182 collapsed 0180/0181 with 2 consumers. ADR 0186 collapsed ADR 0185's inline walks with 2 callers (`collectgarbage()` + auto-trigger). Both were the right call. Don't wait for the third; let two be the trigger when the duplication is structural.
9. **Audit + Tidy First completes a sweep.** ADR 0188's systematic probe-driven audit + `88491b6` refactor turned the TaggedValue source-position sweep from "feature added in 4 ADRs" into "invariant load-bearing across the codegen surface". Sweeping a topic without the closing Tidy First leaves dead code that misleads future readers.
10. **Doc updates are commits too.** The Phase 2 closed declaration (`60932bf`) was a 5-line ADR doc edit. Without it, the project's milestone visibility would be wrong even though all the implementation work was done.

## Next chokepoint candidates (sweep-aware)

Carrying forward from the prior retro and adding the post-sweep state:

- **Phase 3 GC stack walk (ADR 0189+ / ADR 0160 implementation).** Largest remaining structural arc. Worklist capacity (R1 from the pre-flight memo) lives here. Per-type walk strategy completes the `gc_type_meta` table. Stack walk lifts the v1 safety mode; at that point freeing is observable end-to-end and the existing `phase3_gc_*` regression pins become meaningful feature tests. **Multi-session.**
- **Expr-position `Index` → `IndexTagged` widening sub-sweep (ADR 0054 chokepoint).** Still the deepest structural root from the prior retro. Touching ADR 0054 unlocks consumer-kind-aware tagged reads at the language level, which would in turn close the remaining gap on `t1[t2[k]]` shapes the ADR 0188 audit could not reach because the parser doesn't expose them.
- **Cross-procedure inference.** Caller's actual arg kind feeds callee body inference. Generalisation of ADRs 0180/0181/0182/0183's body-walk-decisive merge. Would let `pairs(param)` work without explicit Table-context signal inside the body. Probably plannable as a single ADR after a probe.
- **Bool inference (narrow gate).** Lua truthiness limits applicability; only safe signals are strict equality with Bool literals and `not param`-return-position. Probably small ADR; lower ROI than the above.

Ordering recommendation: **Phase 3 GC stack walk** (continues current arc; closes the v1 safety-mode loop) **OR** **Index widening sub-sweep** (deepest chokepoint; unlocks the next round of Local-only retirements). Both have multi-session shapes. Cross-procedure inference is the small-fits-in-one-session win if a different break is wanted.

## References

- ADRs 0182-0188 (this sweep)
- [Sweep 0166-0177 retrospective](sweep-0166-0177-retrospective.md) — direct precedent; predicted the horizontal-duplication closure that this sweep delivered
- [GC pre-flight review memo](gc-0159-0162-preflight-review.md) — mid-sweep Codex 6 retrospective on the GC arc
- ADR 0054 — flat-f64 `Index` chokepoint (still the next structural root)
- ADR 0133 — Phase 2 closure criteria (now formally closed)
- ADR 0162 — GC auto-trigger design (now implemented as 0186)
- ADR 0179 — `materialize_tagged_source_if_needed` chokepoint (extended by 0187 + 0188)
