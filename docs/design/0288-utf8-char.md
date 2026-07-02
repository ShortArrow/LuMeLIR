# 0288. `utf8.char(n)` single codepoint (N7-17)

- **Status:** Accepted (single-arg; variadic form deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::Utf8Char`; arity `(1, 1)`; param `[Number]`; ret `[String]`.
- ✅ New runtime `lumelir_utf8_char_one(codepoint, out_buf) -> bytes_written`. Encodes 1-4 bytes into the caller-provided buffer per UTF-8 spec.
- ✅ Codegen allocates a 4-byte String obj, calls runtime, rewrites length header to the actual byte count, NUL-terminates. Same shape as `string.gsub` (ADR 0286) and `utf8.char` mirrors that pattern.
- ✅ Invalid codepoint validation in runtime: negatives, values > 0x10FFFF, and the UTF-16 surrogate range 0xD800..=0xDFFF all return byte-count 0 (yields an empty string).
- ❌ Variadic form `utf8.char(n1, n2, ...)` — deferred (needs varargs ABI).

## Tests

5 e2e (`tests/phase4_n7_17_utf8_char.rs`): ASCII `65 → "A"`; `utf8.len(utf8.char(65)) → 1`; 2-byte `U+00E9`; 3-byte `U+3042`; 4-byte `U+1F600`. 1765 → 1770.

## References

- Lua 5.4 §6.5 — `utf8.char`.
- ADR 0277 — `utf8.len` (namespace precedent).
- ADR 0286 — String-obj alloc + rewrite-length pattern reused.
- Roadmap 2026-07-02 — layer-based sizing (runtime + codegen-arm).
