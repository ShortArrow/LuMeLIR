# 0204. `string.format` — Scientific `%e` and Char `%c`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Continuation of ADRs 0202 (`%x`/`%X`) and 0203 (`%o`/`%g`). Adds the next two Lua-spec format specs: scientific notation `%e` and char `%c`.

## Scope (literal)

- ✅ `FormatSpec::Scientific` / `Char` variants.
- ✅ Parser matches `'e'` / `'c'`.
- ✅ Per-arg validation: both require Number.
- ✅ Codegen: `'e'` joins f64 direct-push (with `%f` / `%g`); `'c'` joins f2i + i64 path (with `%d` / `%x` / `%X` / `%o`).
- ❌ Width / precision / flag suffixes. Same scope ceiling as predecessor ADRs.
- ❌ `%i` (alias for `%d` in C99), `%u` (unsigned), `%q` (Lua-style quote). Separate ADRs if needed.

## Decision

`FormatSpec` gains `Scientific` and `Char`. Parser / per-arg validation / codegen dispatch all follow the ADR 0202/0203 pattern.

## Tests

`tests/phase4_string_format_e_c.rs` (NEW, 2 e2e):

1. `string.format("%e", 1.5)` → `"1.500000e+00"`.
2. `string.format("%c", 65)` → `"A"`.

## References

- [Lua 5.4 §6.4](https://www.lua.org/manual/5.4/manual.html#pdf-string.format)
- [ADR 0152](0152-string-format.md), [0202](0202-string-format-hex.md), [0203](0203-string-format-octal-general-float.md).
