# 0280. Pattern anchors `^` / `$` (N4-C)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-28
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `^` at pattern start anchors to s_start = 0. Anywhere else it's a literal `^` (Lua spec).
- ✅ `$` at pattern end anchors to s_end = s.len(). Anywhere else it's a literal `$` (Lua spec).
- ✅ Both anchors combine (`^pat$` = whole-string match).
- ✅ Anchors compose with N4-A classes / `.` / literals and N4-B quantifiers (`^%d+`, `%d+$`).
- ❌ Multi-line `^`/`$` (per-line anchoring) — not in Lua spec, no change planned.

## Implementation

`src/bridge_runtime.rs`:

1. `lumelir_string_match_extents`: detect leading `^`. If present, strip the byte from the pattern slice and constrain the outer search to `start = 0` only (skips the iterate-all-positions loop).
2. `match_pattern`: if remaining pattern is exactly `$` (`p_i + 1 == pat.len()` and `pat[p_i] == b'$'`), succeed iff `s_i == s.len()`. Anywhere else `$` falls through to the literal-atom path.

The outer search becomes:

```
start = 0
stop = if anchored { 0 } else { s_len }
loop {
    if try_match_at(s, start, pat) succeeds: return
    if start >= stop: return 0
    start += 1
}
```

## Tests

9 e2e (`tests/phase4_n4c_anchors.rs`): `^abc` matches at start; rejects elsewhere; `def$` matches at end; rejects elsewhere; `^hello$` whole-string yes / no; `^%d+`; `%d+$`; mid-pattern `$` literal. 1715 → 1724.

## References

- ADR 0231 — pattern engine v1.
- ADR 0279 — quantifiers + recursive backtracking.
- Lua 5.4 §6.4.1 — Patterns (anchors).
- Roadmap 2026-06-27 N4-C.
