# 0286. `string.gsub(s, pat, repl)` string-repl form (N4-G)

- **Status:** Accepted (partial; Function-repl + multi-return `(result, count)` deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `Builtin::StringGsub` arity `(3, 3)`; params `[String, String, String]`; ret `[String]`.
- ✅ New runtime entry `lumelir_string_gsub_string_repl(s_ptr, pat_ptr, repl_ptr, out_buf, out_max) -> i64` (bytes written).
- ✅ Codegen conservatively upper-bounds output at `(s_len + 1) * (repl_len + 1) + repl_len + 8`, allocates a String obj of that size, hands the payload ptr to the bridge, then rewrites the length header + NUL after the call returns the actual byte count.
- ✅ Empty-pattern edge case: input returned unchanged (avoids infinite spin from zero-width matches at every position).
- ✅ Zero-width matches (`%d*` etc.): output byte at match position verbatim + advance by 1.
- ✅ Anchor semantics fix in `run_pattern_from`: `^` in a pattern with `init_byte > 0` returns no match (anchor is anchored to the *string* start, not the search-range start). Same fix benefits ADR 0283 gmatch-loop scenarios.
- ❌ Multi-return `(result, count)` — deferred to a follow-up N4-G-2 ADR.
- ❌ Function `repl` — `string.gsub(s, pat, function(m) return ... end)` — deferred (needs builtin-callable Function values, related to N4-F-2c work).
- ❌ Table `repl` — `string.gsub(s, pat, t)` where `t[capture]` is the replacement — deferred.
- ❌ Backref `%0`-`%9` inside `repl` — deferred. Current impl copies `repl` bytes verbatim; `%0` etc. would need capture-aware expansion.
- ❌ `n` argument limiting substitutions — deferred.

## Tests

8 e2e (`tests/phase4_n4g_gsub.rs`): literal replace; digit class; no-match; `+` quantifier; set `[aeiou]`; anchor `^abc` limits to first; empty replacement (delete); replacement longer than match. 1750 → 1758.

## References

- ADR 0231 — pattern engine v1.
- ADRs 0278-0285 — N4 sub-task progression.
- Lua 5.4 §6.4 — `string.gsub`.
- Roadmap 2026-06-27 N4-G.
