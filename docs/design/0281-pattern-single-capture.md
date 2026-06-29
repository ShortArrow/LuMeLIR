# 0281. Pattern single capture `(...)` (N4-D)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-30
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Pattern engine recognises `(` and `)` and records one capture's (start, end) byte range in a threaded `CapState`.
- ✅ Snapshot-restore on every failed try (covers both atom-level backtracking and quantifier backtracking via `match_max` / `match_opt`).
- ✅ New runtime entry point `lumelir_string_match_capture1`: returns the first capture's packed `(len, pos)` when the pattern contains a capture, else the overall match extent (Lua spec for `string.match`).
- ✅ `string.match` codegen switched to the new entry point.
- ✅ `string.find` continues to use `lumelir_string_match_extents` (overall extent, unchanged).
- ❌ Multiple captures + multi-return — deferred. Currently any `(` after a still-open `(` short-circuits the match (nested captures unsupported).
- ❌ Position capture `()` — deferred.
- ❌ Back-references `%1`-`%9` — deferred (independent of N4-D scope).
- ❌ Captures in `gmatch` / `gsub` — N4-F / N4-G.

## Algorithm

`CapState` is `Copy`:

```rust
struct CapState {
    has_pending: bool,
    pending_start: usize,
    captured: bool,
    cap_start: usize,
    cap_end: usize,
}
```

`match_pattern` arms for `(` and `)`:

```
(   → if has_pending: fail (no nesting); save state; set pending start = s_i;
      recurse; on failure restore.
)   → if !has_pending: fail; save; commit capture (start = pending_start,
      end = s_i); recurse; on failure restore.
```

`match_max` and `match_opt` snapshot `*cap` before each recursive try and restore on failure. This preserves the invariant that a failed sub-match never leaks its CapState into the next try.

## Tests

7 e2e (`tests/phase4_n4d_captures.rs`): inner-digit capture; empty capture under `*`; capture at pattern start; capture with `^` anchor; no-capture returns full match; capture with `$` anchor; no-match returns nil. 1724 → 1731.

## References

- ADR 0231 — pattern engine v1.
- ADR 0278 — `string.match` wire-up.
- ADR 0279 — quantifiers (`match_max` / `match_opt` reuse).
- ADR 0280 — anchors.
- Lua 5.4 §6.4.1 — Patterns (captures).
- Roadmap 2026-06-27 N4-D.
