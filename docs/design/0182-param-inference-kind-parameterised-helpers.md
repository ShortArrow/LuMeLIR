# 0182. Param-Kind Body-Walker — Kind-Parameterised Helpers (Tidy First)

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-06-09
- **Deciders:** ShortArrow

## Context

[ADR 0180](0180-param-table-context-inference.md) added `mark_ident_as_table` and the body-decisive merge rule `Function(_) | Table`. [ADR 0181](0181-param-string-context-inference.md) folded String in with a direct copy: `mark_ident_as_string` plus a merge widened to `Function(_) | Table | String`.

Two kinds is the point at which the duplication is no longer "two examples" — it is a pattern. Adding a third kind (e.g. ADR 0183's MethodCall String fold-through, or any future Bool / Number-context inference) would mean a third near-identical helper and a third merge-arm edit. The CLAUDE.md 第3原則 (non-ad-hoc / Tidy First) prescribes consolidating before adding the third row, not after.

## Scope (literal)

- ✅ Collapse `mark_ident_as_table` + `mark_ident_as_string` into a single `mark_ident_as(expr, name_to_idx, kinds, kind)`.
- ✅ Extract the body-decisive merge predicate into `is_body_decisive_kind(kind: ValueKind) -> bool`.
- ✅ Update all call-sites in `infer_param_kinds` and the `LowerCtx::for_function` merge.
- ✅ Behaviour-preserving: zero test delta (1407 stay green).
- ❌ No new inference signal. Adding MethodCall String / Bool / etc. is a follow-up ADR consuming this refactor.
- ❌ `is_table_consumer_builtin` / `is_string_method_callee` predicates left as-is — they encode kind-specific knowledge (which builtins consume which kind) and do not collapse cleanly.
- ❌ `Function(_)` callee-position arm in the Call match is NOT folded — `Function(arity)` carries arity payload distinct from `mark_ident_as`.

## Decision

### `src/hir/mod.rs`

1. **Replace two markers with one**:
   ```rust
   fn mark_ident_as(
       expr: &Expr,
       name_to_idx: &Map<&str, usize>,
       kinds: &mut [ValueKind],
       kind: ValueKind,
   ) {
       if let ExprKind::Ident(name) = &expr.kind
           && let Some(&idx) = name_to_idx.get(name.as_str())
       {
           kinds[idx] = kind;
       }
   }
   ```
   Delete `mark_ident_as_table` and `mark_ident_as_string`.

2. **Extract merge predicate**:
   ```rust
   fn is_body_decisive_kind(kind: ValueKind) -> bool {
       matches!(
           kind,
           ValueKind::Function(_) | ValueKind::Table | ValueKind::String
       )
   }
   ```
   Lives next to `infer_param_kinds` (module-private, same file).

3. **Merge site at `LowerCtx::for_function`** (line ~2128):
   ```rust
   let kind = if is_body_decisive_kind(body_kinds[i]) {
       body_kinds[i]
   } else {
       external_kinds[i]
   };
   ```

4. **All marking call-sites** pass an explicit `ValueKind::Table` or `ValueKind::String` argument — the body-walker arms remain otherwise identical.

### Tests

No new tests. The existing 1407-test corpus — including the 4 ADR 0180 e2e and 3 ADR 0181 e2e — is the regression net.

## Alternatives considered

- **Defer until 3rd kind arrives.** Rejected — the cost to refactor now (2 helpers → 1, 1 merge match → 1 predicate) is smaller than the cost of editing all three sites again in ADR 0183. Tidy First is precisely about doing this before the third repetition.
- **Generic over a closure `mark_if_ident(expr, ..., |idx| kinds[idx] = K)`.** Rejected — strictly more code for the same effect; the kind enum value is already a first-class argument.
- **Encode the merge predicate as an `impl ValueKind` method.** Rejected — `ValueKind` is defined in `hir/ir.rs` and the "body-decisive" property is local to param inference; an `ir.rs` method would leak inference-pass concerns into the type definition.

## Consequences

**Positive**
- Adding a new kind (e.g. ADR 0183 MethodCall String) becomes a one-line arm extension: `mark_ident_as(receiver, ..., ValueKind::String)`.
- The merge predicate has one truth-site instead of one duplicated literal.
- The duplication signal that prompted Codex 第3原則 review is silenced before the third occurrence.

**Negative**
- One extra parameter on the call-site (the kind itself). Trivially readable.
- Future kinds whose marking logic diverges from "set `kinds[idx] = K` if ident matches" would have to bypass the helper. Acceptable — the helper does exactly the common shape; divergent kinds add their own arm.

**Locked in until superseded**
- `is_body_decisive_kind` membership is the source of truth for the merge rule. Future kind additions must amend it.

## Documentation updates

- [x] §8 — adds 0182.
- [x] ADR 0180 / 0181 cross-reference — refactor consumes their pattern.

## Test count delta

```
Step 0: 1407 (after ADR 0181)
C1 (doc): 1407 → 1407
C2 (refactor): 1407 → 1407 (behaviour-preserving)
```

## Critical files

- `src/hir/mod.rs`:
  - Replace two markers with `mark_ident_as`.
  - Add `is_body_decisive_kind` predicate.
  - Update body-walker call-sites + `LowerCtx::for_function` merge.

## Risks

| Risk | Mitigation |
|---|---|
| Refactor introduces a subtle behaviour change at the merge site | The literal expansion `Function(_) | Table | String` is preserved verbatim in the predicate body. Existing 1407 tests pin behaviour. |
| Future kind forgets to extend `is_body_decisive_kind` | Single chokepoint — the next ADR adding a body-decisive signal touches one predicate, not two literals across the file. Easier to find via grep. |
| `&mut [ValueKind]` slice type drift | All current call-sites already pass `&mut [ValueKind]` (clippy fix during 0180). Unchanged. |

## Future work

- ADR 0183 = MethodCall form `s:upper()` String inference — direct consumer of this refactor; expected to be a single arm + reuse.
- Bool / Number-context inference (still gated on Lua truthiness analysis — see ADR 0181 Non-goals).
- Cross-procedure / call-site bidirectional inference.

## References

- [ADR 0180](0180-param-table-context-inference.md) — introduces the marker + merge pattern.
- [ADR 0181](0181-param-string-context-inference.md) — first fold-through, surfaces duplication.
- CLAUDE.md 第3原則 — non-ad-hoc / Tidy First mandate.
