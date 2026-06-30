# 0282. Pattern character sets `[...]` (N4-E)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-30
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Literal-character sets `[abc]`.
- ✅ Negation `[^abc]` (leading `^` inside the brackets only).
- ✅ Ranges `[a-z]`, `[0-9]`, `[A-Za-z_]` (multiple ranges + literals mix freely).
- ✅ Classes inside sets `[%d%s]`, `[^%s]`.
- ✅ `%X` escape inside the set so `%]` doesn't close the set.
- ✅ Composes with N4-B quantifiers (`[abc]+`, `[aeiou]?`), N4-C anchors, N4-D capture (`([0-9]+)`).
- ❌ Leading `]` as a literal set member (Lua oddity) — deferred.
- ❌ `-` as a literal first/last set member — deferred.
- ❌ Class complements abbreviated inside set (e.g., `[%A]`) — already works via `matches_class('A')` because complements are handled there.

## Implementation

Two new helpers in `src/bridge_runtime.rs`:

- `atom_len` extended so `[` returns the byte distance to the matching `]` (walking past `%X` so a literal `%]` doesn't close).
- `matches_atom` extended so `[` dispatches to a new `matches_set`.
- `matches_set` walks the body once per test character, decoding `%X` classes, `b-X` ranges (X not `]`), and literal characters. Leading `^` toggles the negate flag and the final result is XOR'd with it.

Set bodies are opaque to the outer `match_pattern` — `(` / `)` / `*` etc. inside a set are not seen by the recursive matcher because the set spans a single atom whose length is reported by `atom_len`.

## Tests

10 e2e (`tests/phase4_n4e_sets.rs`): literal set with `+`; negation `[^aeiou]`; lowercase range; digit range; class-in-set `[%d%s]+`; combined `[A-Za-z_]+`; negated class `[^%s]+`; `?` quantifier; set inside capture `([0-9]+)`; nil case. 1731 → 1741.

## References

- ADR 0231 — pattern engine v1.
- ADR 0279 — quantifiers (compose unchanged).
- ADR 0280 — anchors.
- ADR 0281 — single capture.
- Lua 5.4 §6.4.1 — Patterns (character sets).
- Roadmap 2026-06-27 N4-E.
