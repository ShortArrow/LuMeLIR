# 0279. Pattern quantifiers `*` `+` `?` (N4-B)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-28
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Greedy `*` (0+), `+` (1+), `?` (0 or 1) on any single-byte atom (literal, `.`, `%X` class).
- ✅ Pattern engine refactored from iterative one-token-at-a-time match to recursive backtracking (`match_pattern` + `match_max` + `match_opt` in `src/bridge_runtime.rs`).
- ✅ All slice accesses go through `unsafe { get_unchecked }` because the `#![no_std]` bridge object cannot resolve `core::panicking::panic_bounds_check`. Each access site bounds-checks first; SAFETY contracts documented inline.
- ❌ Non-greedy `-` (Lua's lazy match) — deferred. Lua spec actually has `-` as the lazy 0+ quantifier; that variant is a separate small addition.
- ❌ Set quantifiers `[abc]+` — N4-E lands sets first.
- ❌ Anchors `^` `$` — N4-C.
- ❌ Captures — N4-D.

## Algorithm

```
match_pattern(s, s_i, pat, p_i) =
    if p_i >= |pat|: Some(s_i)
    atom = pat[p_i..p_i + atom_len(p_i)]
    after_atom = p_i + atom_len(p_i)
    if pat[after_atom] in {'*','+','?'}:
        delegate to match_max / match_opt
    else:
        require s_i < |s| && matches_atom(s[s_i], atom)
        match_pattern(s, s_i + 1, pat, after_atom)

match_max(min) = consume greedily, then back off until rest matches
match_opt = try-one-then-zero
```

Recursion depth is bounded by pattern length (no left-recursion possible because every recursive call advances `p_i` by at least the atom length when there's no quantifier, and the quantifier branches all advance past the quantifier byte).

## Tests

9 e2e (`tests/phase4_n4b_quantifiers.rs`): `%d*` empty match; `%d+` over `"abc42def"` → `"42"`; `%d+` nil; `%d?` empty; `%d?:` over `"x:7"` → `":"`; `.*z` backtrack to last `z`; `%d+5` backtracks to satisfy trailing literal; `.*` matches all; `%a+%d?` mixed. 1706 → 1715.

## References

- ADR 0231 — pattern engine v1 (classes + `.` + `%X` escape).
- ADR 0278 — `string.match` wire-up.
- Lua 5.4 §6.4.1 — Patterns.
- Roadmap 2026-06-27 N4-B.
