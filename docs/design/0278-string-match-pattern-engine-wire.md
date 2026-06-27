# 0278. `string.match` wires up the pattern engine (N4-A)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-27
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `Builtin::StringMatch` codegen switched from `lumelir_string_find_plain` (literal substring) to `lumelir_string_match_extents` (ADR 0231 pattern engine).
- ✅ Decodes packed result (low 32 bits = 1-indexed start, high 32 bits = matched byte length).
- ✅ Builds a fresh `String` object via `emit_string_obj_from_bytes` from the matched byte range (so the returned value is the *actual* matched substring, not the pattern source).
- ✅ Patterns supported through this wire-up (all already shipped in `lumelir_string_match_extents` per ADR 0231): literal char, `.` wildcard, classes `%a %d %s %w %p` (+ complements `%A %D %S %W %P`), additional classes `%l %u %x %c` already covered, magic-char escape `%X`.
- ❌ Quantifiers (`*` `+` `?`) — N4-B.
- ❌ Anchors (`^` `$`) — N4-C.
- ❌ Captures `(...)` — N4-D.
- ❌ Sets `[abc]` `[^abc]` `[a-z]` — N4-E.
- ❌ `string.gmatch` iterator — N4-F.
- ❌ `string.gsub` — N4-G.

## Why this is N4-A scope

Roadmap [2026-06-27](../notes/roadmap-2026-06-27-rebuild.md) lists N4-A as "Pattern class基本: `%a %d %s %w %p` + literal + `.`". The engine itself shipped under ADR 0231 (M7-D) but only `string.find` was wired up. `string.match` still returned the *pattern* on a literal-only `find_plain` call, which is wrong as soon as the pattern contains any class or wildcard. This ADR finishes the wire-up: `string.match` returns the matched substring for all class/wildcard/literal patterns.

## Tests

9 e2e (`tests/phase4_n4a_pattern_classes.rs`): digit, alpha, wildcard, whitespace, word-vs-punct (nil case), punct, multi-char `%d%d`, literal, mixed class+literal. 1697 → 1706.

## References

- ADR 0231 — `lumelir_string_match_extents` pattern engine (classes + `.` + `%X`).
- ADR 0228 — `string.find` literal substring (superseded for `string.match`).
- ADR 0230 — `string.match` literal-only initial wiring (superseded).
- Roadmap 2026-06-27 — N4 sub-task decomposition.
