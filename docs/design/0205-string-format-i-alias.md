# 0205. `string.format` — `%i` Alias for `%d`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Trivial extension of the ADR 0152 / 0202 / 0203 / 0204 format-spec set. C99 added `%i` as a printf alias for `%d`; both produce the same output. Lua follows C's behaviour. Supporting `%i` removes a small portability friction.

## Scope (literal)

- ✅ Parser maps `'i'` to `FormatSpec::Decimal` (no new variant; pure alias).
- ✅ Codegen rewrites `%i` → `%lld` (same as `%d`).
- ✅ Per-arg dispatch: `'i'` joins `'d'` in the f2i + i64 push path.
- ❌ No new variant — alias only.

## Decision

`parse_format_specifiers` matches `b'd' | b'i'` → `FormatSpec::Decimal`. Codegen printf-format builder and per-arg dispatch loop accept `'i'` alongside `'d'`.

## Tests

`tests/phase4_string_format_i.rs` (NEW, 1 e2e):

`string.format("%i", 42)` → `"42"`.

## References

- [Lua 5.4 §6.4](https://www.lua.org/manual/5.4/manual.html#pdf-string.format)
- C99 standard, §7.19.6.1 — `%i` printf conversion.
