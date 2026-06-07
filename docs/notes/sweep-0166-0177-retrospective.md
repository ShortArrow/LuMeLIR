# Sweep 0166-0177 Retrospective (Codex 6 Lens)

- **Date:** 2026-06-07
- **Scope:** ADRs 0166 → 0177 (12 consecutive, 36 commits, 3-commit cycle each)
- **Sweep delta:** tests 1362 → 1399 (+37), `src/codegen/emit.rs` 約 +900 LOC, `src/hir/mod.rs` 約 +30 LOC
- **Authoring cadence:** doc (C1) → Red Day 0 (C2) → impl + push (C3)

## What landed

| ADR | Theme | Side | Key shape |
|---|---|---|---|
| 0166 | Number-key `__index` Function form | read | Function-form metamethod dispatch |
| 0167 | Multi-hop Number-key `__index` | read | depth-bounded recursion (`METATABLE_INDEX_MAX_HOPS = 8`) |
| 0168 | Number-key `__newindex` Table form | write | extracted `emit_array_index_assign_at` |
| 0169 | Number-key `__newindex` Function form | write | mirror of 0166 |
| 0170 | Multi-hop Number-key `__newindex` | write | extracted `emit_number_key_indexassign_routed` |
| 0171 | Mid-array TAG_NIL `__newindex` trigger | write | `probe_mid_nil` scf.if OR `key_high` |
| 0172 | `rawset(t, n, v)` Number key | write | open up ADR 0136 deferral |
| 0173 | `rawget(t, n)` Number key | read | mirror of 0172 |
| 0174 | `rawget(t, k)` Local(TaggedValue) key | read | runtime tag dispatch |
| 0175 | `rawset(t, k, v)` Local(TaggedValue) key | write | mirror of 0174 |
| 0176 | `rawset` Local(TaggedValue) key + value | write | Local(TaggedValue) value substitution (ADR 0139 slot-ptr convention) |
| 0177 | `t[k]` Local(TaggedValue) key (IndexTagged) | read | language-level chokepoint |

Closed deferral rows:
- ADR 0135 "Number-key array writes bypass `__newindex`" (0168)
- ADR 0136 "Number-key forms deferred" for raw-builtins (0172/0173)
- "TaggedValue runtime-key dispatch" (recurring across 0142/0144/0146/0147/0150/0166/0171/0173) for Local source (0174/0175/0176/0177)

## Codex 6 lens

### #1 non-ad-hoc / Tidy First — **mixed**
Within each ADR, Tidy First was respected: `emit_array_index_assign_at` (0168), `emit_number_key_indexassign_routed` (0170), `emit_indextagged_number_key_dispatch` (0177) were extracted at the rule-of-three trigger. **But horizontally**, the same _tagged-key dispatcher shape_ was repeated across 5 sites without a shared helper. See §"Horizontal duplication" below.

### #2 TDD — **clean**
Every ADR followed Red Day 0 → impl → Green. No log-driven verification, no skipped red. GPG-signing hiccups (twice) did not bypass the cycle.

### #3 FP / responsibility separation — **clean**
HIR `lower_builtin_call` arg-validation arms (0172-0175) added as additive `_ok` predicates, never relaxing the floor (`if !(hash_ok || ...) { TypeMismatch }`). HIR stayed pure; emit stayed effectful.

### #4 Clean Architecture / dependency direction — **clean**
All 12 ADRs respect HIR → codegen direction. Slot-ptr-as-value convention (ADR 0139) consistently threaded through 0176 instead of inventing a parallel ABI.

### #5 Given/When/Then state diff — **clean**
Each ADR carries an explicit scope literal (✅/❌). The "what runs through the new path" answer is unambiguous in every doc.

### #6 Naming / docstring — **clean**
Extracted helpers carry their intent in the name (`emit_indextagged_number_key_dispatch`, `emit_array_index_assign_at`, `emit_number_key_indexassign_routed`). In-body comments stayed minimal.

## Horizontal duplication

The repeated shape across **5 consumer sites** (rawget Number / rawget Tagged / rawset Number / rawset Tagged / IndexTagged Tagged):

```text
Nil trap on tag (s_table_index_nil)
scf.if(tag == TAG_NUMBER)
  then: bitcast i64→f64 → NaN trap → f2i → <kind-specific Number arm>
  else: <kind-specific hash arm>   (read = hash_lookup + __index fallback; rawset = hash_indexassign skip_metatable=true)
```

Concrete grep evidence (`src/codegen/emit.rs`):
- Read-side (IndexTagged, ~5430): Nil trap → tag check → Number arm (`emit_indextagged_number_key_dispatch`) / hash arm (`emit_hash_lookup_into_tagged_slot` + `emit_metatable_index_fallback_if_nil`)
- Read-side (rawget, ADR 0174): same scaffold; hash arm skips `__index` fallback
- Write-side (rawset Tagged, ADR 0175 + 0176): same scaffold; Number arm calls `emit_array_index_assign_at`; hash arm calls `emit_hash_indexassign_with_newindex(skip_metatable: true)`

**The dispatcher scaffold is the same; only the Number-arm and hash-arm closures differ.** Today each call site re-builds the scf.if and the trap boilerplate inline (60-80 LOC × 5).

## Concrete helper proposal (next ADR — 0178)

```rust
fn emit_tagged_key_runtime_dispatch<'a, 'c, FN, FH>(
    context: &'c Context,
    block: &'a Block<'c>,
    source_slot: Value<'c, 'a>,           // (tag, payload) i.e. slots[idx]
    types: &Types<'c>,
    loc: Location<'c>,
    nil_trap_global: &str,                // s_table_index_nil | s_rawset_nil_key | ...
    nan_trap_global: &str,                // s_table_index_nan | s_rawset_nan_key | ...
    on_number_key: FN,                    // FnOnce(&Block, key_i: Value) — Number arm body
    on_hash_key: FH,                      // FnOnce(&Block) — hash arm body (uses source_slot)
)
where
    FN: FnOnce(&Block<'c>, Value<'c, '_>),
    FH: FnOnce(&Block<'c>),
```

Body = the literal scaffold above. The 5 call sites collapse to ~10 LOC each. Future ADRs (Non-Local source materialisation, flat-f64 widening) add to one helper, not five.

**Risk:** trap message wording diverges per site (e.g. `s_rawset_nil_key` vs `s_table_index_nil`). The helper takes the global name as a parameter; the message stays site-local.

## Lessons (durable)

1. **Rule of three is necessary but not sufficient.** Within-ADR Tidy First catches local duplication; cross-ADR shape repetition requires a periodic sweep retro to surface.
2. **Scope-literal discipline pays for itself.** ✅/❌ tables made every "deferred to next ADR" tractable and let the sweep land without scope-creep.
3. **Slot-ptr-as-value (ADR 0139) is the right abstraction for TaggedValue source threading.** No ADR in this sweep needed to invent a competing convention.
4. **Codex 6 #1 has two layers** — per-ADR (caught) and per-sweep (missed until now). Future sweeps should budget one retrospective per ~10 ADRs.

## Next moves (planned at retro time)

- **ADR 0178 = tagged-key dispatcher helper** (behaviour-preserving Tidy First; 5 call sites → 1 helper). Test delta 0 (pure refactor; existing tests pin).
- **ADR 0179 = Non-Local TaggedValue source** (tmp-slot materialisation). Sits on top of the helper — adds one new entry path, not five.
- **Flat-f64 `Index` widening** — orthogonal layer (consumer kind), defer until after 0178/0179.

## Fold-through (ADRs 0178 + 0179)

Both planned moves landed; updates worth noting for the next sweep:

### ADR 0178 — what actually happened
- Initial scan showed **3 sites**, not 5: IndexTagged TaggedValue arm (~5430), rawset Tagged sub-arm (~10963), rawget Tagged sub-arm (~11191). The retro had over-counted; the 5-count above conflated the dispatcher (this shape) with the Number-key arms (different shape; not part of the unification).
- Extraction used `for<'a>`-free HRTB closures (`FnOnce(&Block<'c>, Value<'c, '_>)`) which compiled cleanly — the looser bound was sufficient because melior 0.27's `Value` lifetime parameters are permissive across nested-block borrows in practice.
- Net `src/codegen/emit.rs` **−105 LOC** (+273 / −378).
- Per-site fences (Nil-trap for IndexTagged, too_small for rawset, in-range scf.if for rawget) stayed outside the helper, exactly as planned. No surprises.

### ADR 0179 — what actually happened
- Scope **narrowed during drafting** from 4 → 3 source positions. The pre-flight check found `HirExprKind::Index` infers as `Number` (`src/hir/mod.rs:180`), not TaggedValue, so `t1[t2[k]]` IndexTagged-key non-Local case is unreachable through the parser today. The IndexTagged-key non-Local case is now a future ADR blocked on **expr-position `Index` → `IndexTagged` widening** (touches the ADR 0054 chokepoint).
- Test-writing surfaced a **second structural blocker**: function parameters default to `Number` kind, so `for k, _ in pairs(param) do ... end` inside a helper fails with `for-in-pairs: lhs=table rhs=number`. Worked around by using upvalue-captured tables. The blocker is a separate roadmap item.
- HIR pre-pass approach worked exactly as planned. ADRs 0174/0175/0176 Local-only clauses dropped uniformly; the obsolete arg[2] rejection block removed.
- Test delta 1397 → 1400 (3 e2e Red→Green); existing 1397 unchanged.

### Lessons added

5. **Doc-first ADRs need a pre-flight code probe.** ADR 0179 was drafted with 4 positions; only after writing tests did it become clear one of them was unreachable through the parser. A 15-min `infer_kind` / parser-shape probe before C1 saves a C1-amend commit. (Cost this time: one extra commit `2969c96`. Cheap enough that the lesson stands, not a process change.)
6. **Per-sweep duplication is sometimes mis-counted from the inside.** The "5 sites" figure in this retrospective was wrong; only 3 sites had the dispatcher shape. A retro should cite line numbers AND `grep` evidence; future retros should re-grep at write time, not work from memory.

## Next chokepoint candidates

The structural root behind every "TaggedValue source" deferral row is **ADR 0054**: `Index` infers as `Number` in expression position. This forces TaggedValue inputs to come through narrow channels (Local, LocalInit-widening), which is why dispatchers exist at all and why ADR 0179 had to narrow.

Roadmap-level candidates (need own decision docs):

- **Expr-position `Index` → `IndexTagged` widening.** Touches the ADR 0054 chokepoint. Probably needs to be split: (a) HIR rule change with feature-flag-style coverage, (b) per-consumer kind reconciliation as widening lands. Expected to touch many call sites — should be planned as a multi-ADR sub-sweep, not one ADR.
- **Function parameter kind inference (or annotation).** Unblocks `pairs(param)` and idiomatic helpers. Two paths: (i) parameters default to TaggedValue, (ii) infer from call sites. Either way it's a HIR-level decision with codegen consequences.
- **IndexAssign value-side non-Local TaggedValue.** The remaining write-side deferral row. Direct mirror of ADR 0179 on a different code path; small and tractable but doesn't open new ground.

Ordering recommendation: **parameter inference first** (smallest, unblocks test patterns), **then plan the Index-widening sub-sweep** (multi-ADR), **then IndexAssign value side fold-through**.

## References

- ADRs 0166-0177 (this sweep)
- ADR 0054 — flat-f64 `Index` chokepoint (next structural root)
- ADR 0084 — TaggedValue Local restriction precedent
- ADR 0088 — `emit_hash_lookup_into_tagged_slot` chokepoint
- ADR 0134 / 0150 — `__index` hash-key fallback chain
- ADR 0139 — slot-ptr-as-value convention
- ADR 0165 — Number-key `__index` array OOB fallback
- [ADR 0178](../design/0178-tagged-key-runtime-dispatch-helper.md) — fold-through helper landed
- [ADR 0179](../design/0179-non-local-tagged-source-materialisation.md) — fold-through materialisation landed
