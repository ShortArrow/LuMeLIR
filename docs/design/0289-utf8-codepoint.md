# 0289. `utf8.codepoint(s, i)` single position (N7-18)

- **Status:** Accepted (single-position; range form deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::Utf8Codepoint`; arity `(2, 2)`; params `[String, Number]`; ret `[Number]`.
- ✅ Runtime `lumelir_utf8_codepoint_one(s_ptr, i_1based)` decodes 1-4 UTF-8 bytes at `s[i-1..]` and returns the codepoint (or -1 on error).
- ✅ Codegen calls runtime and `sitofp`'s the i64 result to f64 for the Number ABI.
- ❌ Range form `utf8.codepoint(s, i, j)` returning multiple codepoints — deferred (needs multi-return).
- ❌ Optional `i` default (Lua spec: default 1) — deferred; call must be 2-arg today.
- ❌ Invalid-UTF-8 propagation as a Lua error — currently returns `-1` as Number; a future ADR should raise proper Lua error.

## Tests

5 e2e (`tests/phase4_n7_18_utf8_codepoint.rs`): ASCII at position 1 (`"A" → 65`); ASCII at position 2 (`"B" → 66`); roundtrip `utf8.codepoint(utf8.char(0x3042), 1) → 12354`; 2-byte `U+00E9 → 233`; 4-byte `U+1F600 → 128512`. 1770 → 1775.

## References

- Lua 5.4 §6.5 — `utf8.codepoint`.
- ADR 0288 — `utf8.char` (namespace + alloc pattern).
- Roadmap 2026-07-02 — layer-based sizing (runtime + codegen-arm).
