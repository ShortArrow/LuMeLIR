# 0287. Pattern `%bxy` balanced + `%f[set]` frontier (N4-H)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `%bxy` balanced-match: scans forward from `s_i`, requires `s[s_i] == x`, tracks a depth counter (`+1` on `x`, `-1` on `y`) until depth returns to 0, then continues the pattern from `p_i + 4`. Fails if `s` ends before depth returns to 0.
- ✅ `%f[set]` frontier: zero-width. Matches iff `prev` char is **not** in `set` and `cur` char **is** in `set`. Beginning/end of `s` treated as `\0` per Lua spec. Consumes 0 bytes of `s`, advances `p_i` past the `[...]`.
- ✅ Both compose with existing anchors (`^` / `$`), captures `(...)`, and quantifiers (only where meaningful — quantifiers on `%f` are undefined per Lua spec; on `%b` they behave as the balanced span acting as one atom).
- ❌ Quantifiers `+`/`*`/`?` on `%b` / `%f` are not specially handled; the engine treats the balanced span as a whole atom for downstream matching but does not allow the atom itself to be quantified. Matches Lua reference impl behaviour.

## Implementation

`src/bridge_runtime.rs`:

- Two special-case branches inserted at the top of `match_pattern` (before the atom / quantifier flow):
  - `%b` → `match_balanced(x, y)` which advances `s_i` past the balanced span and recurses on the rest.
  - `%f[set]` → `is_frontier(s, s_i, set)` gate + zero-width recurse.
- `match_balanced` reuses no atom-flow helpers; direct byte compare on `x`/`y`.
- `is_frontier` reuses `matches_set` (ADR 0282) so class syntax inside the set works (`%f[%w]`, `%f[%W]`, `%f[%a%d]`).

## Tests

7 e2e (`tests/phase4_n4h_balanced_frontier.rs`): simple `%b()`; nested `%b()`; `%b{}`; unbalanced (nil); `%f[%w]` word-boundary via gsub `|` insertion; `%f[%W]` end-of-word transition; `%f[%a]` matches at position 0 when first char is alpha. 1758 → 1765.

## References

- ADR 0231 — pattern engine v1.
- ADR 0282 — character sets (`matches_set` reuse).
- ADRs 0278-0286 — N4 sub-task progression.
- Lua 5.4 §6.4.1 — Patterns (`%b`, `%f`).
- Roadmap 2026-06-27 N4-H.
