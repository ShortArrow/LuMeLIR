# 0283. `string.find(s, pat, init)` 3-arg form (N4-F-1)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-30
- **Deciders:** ShortArrow

## Scope re-pin

The roadmap [2026-06-27](../notes/roadmap-2026-06-27-rebuild.md) lists N4-F as `string.gmatch` iterator with a **2-session** budget. The closure-returning iterator form requires either:

1. Generic-for protocol extension (currently parser only accepts `ipairs` / `pairs` / explicit 3-tuple), AND
2. Builtin-returned closure synthesis (no precedent — closures today only come from user `function` literals).

Both items are structurally larger than a single session. This ADR is the **first half** (N4-F-1): the runtime + `string.find` extension that the closure form will be built on. N4-F-2 will land the closure / iterator wiring.

## Scope (literal)

- ✅ `Builtin::StringFind` arity widened (2, 2) → (2, 3); 3rd arg is the 1-indexed init position (Lua spec).
- ✅ New runtime entry `lumelir_string_find_init(s_ptr, pat_ptr, init_1based) -> i64` — same packed `(len << 32) | pos` result shape, but searches from `init_1based`.
- ✅ Pattern engine refactor: `run_pattern_from(s_ptr, pat_ptr, init_byte)` is the new core; `run_pattern` is a thin wrapper at `init_byte = 0`.
- ✅ Codegen for both single-return and multi-return `string.find` arms dispatches to `find_init` when 3 args present, else the legacy `match_extents` entry.
- ✅ Enables manual gmatch loops (`while string.find(s, pat, pos) ... pos = end + 1`).
- ❌ `string.gmatch(s, pat)` closure-returning form — N4-F-2.
- ❌ `init` argument bounds-checking / negative-index semantics — current impl clamps `init <= 1` to byte 0 and treats `init > len(s)` as no-match. Negative init (Lua spec: index from end) deferred.

## Tests

6 e2e (`tests/phase4_n4f_find_init.rs`): init skips earlier match; init no-match; init=1 first match; init past first → second; init multi-return (start, end); manual gmatch loop reproduces `string.gmatch("1 22 333", "%d+")` semantics. 1741 → 1747.

## References

- ADR 0228 — `string.find(s, sub)` literal-only origin.
- ADR 0229 — `string.find` multi-return (start, end).
- ADR 0231 — pattern engine.
- ADRs 0278-0282 — N4 sub-task progression.
- Lua 5.4 §6.4 — `string.find`.
- Roadmap 2026-06-27 N4-F.
