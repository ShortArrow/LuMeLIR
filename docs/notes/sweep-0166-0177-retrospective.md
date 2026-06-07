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

## Next moves

- **ADR 0178 = tagged-key dispatcher helper** (behaviour-preserving Tidy First; 5 call sites → 1 helper). Test delta 0 (pure refactor; existing tests pin).
- **ADR 0179 = Non-Local TaggedValue source** (tmp-slot materialisation). Sits on top of the helper — adds one new entry path, not five.
- **Flat-f64 `Index` widening** — orthogonal layer (consumer kind), defer until after 0178/0179.

## References

- ADRs 0166-0177 (this sweep)
- ADR 0084 — TaggedValue Local restriction precedent
- ADR 0088 — `emit_hash_lookup_into_tagged_slot` chokepoint
- ADR 0134 / 0150 — `__index` hash-key fallback chain
- ADR 0139 — slot-ptr-as-value convention
- ADR 0165 — Number-key `__index` array OOB fallback
