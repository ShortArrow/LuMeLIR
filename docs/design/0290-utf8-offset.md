# 0290. `utf8.offset(s, n)` from start (N7-19)

- **Status:** Accepted (2-arg from-start; 3-arg with explicit start deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::Utf8Offset`; arity `(2, 2)`; params `[String, Number]`; ret `[Number]`.
- ✅ Runtime `lumelir_utf8_offset_from_start(s_ptr, n)` walks bytes counting non-continuation ones (top 2 bits ≠ `0b10`) plus the past-end boundary. When the count reaches `n`, returns that 1-indexed byte position.
- ✅ Boundary: `n = codepoint_count + 1` returns `len(s) + 1` (position past the end).
- ✅ `n = 0` returns 1 (Lua spec's alignment-of-current-codepoint interpretation for byte 1).
- ❌ 3-arg form `utf8.offset(s, n, i)` with explicit start byte — deferred.
- ❌ Negative `n` (walk backward from the end) — currently returns -1.

## Tests

5 e2e (`tests/phase4_n7_19_utf8_offset.rs`): first codepoint is byte 1; second ASCII is byte 2; after a 2-byte codepoint the next starts at byte 3; end boundary; `n=0 → 1`. 1775 → 1780.

## References

- Lua 5.4 §6.5 — `utf8.offset`.
- ADR 0277 — `utf8.len` codepoint-count precedent (same non-continuation-byte scan).
- ADRs 0288/0289 — utf8.char / utf8.codepoint sibling.
- Roadmap 2026-07-02 — layer-based sizing (runtime + codegen-arm).
