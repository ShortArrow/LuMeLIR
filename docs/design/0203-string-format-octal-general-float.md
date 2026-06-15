# 0203. `string.format` — Octal `%o` and General Float `%g`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

ADRs 0152 and 0202 covered `%d / %f / %s / %% / %x / %X`. Bucket B real gap: octal `%o` and general-float `%g` are the next-most-common format specs in everyday Lua code. Both are trivial extensions of the existing dispatch.

## Scope (literal)

- ✅ `FormatSpec::Octal` / `GeneralFloat` variants.
- ✅ Parser matches `'o'` / `'g'`.
- ✅ Codegen rewrites `%o` → `%llo` (i64 path, joins `%d`/`%x`/`%X`); `%g` → `%g` (f64 path, joins `%f`).
- ❌ Other Lua-spec format specs (`%e`, `%c`, `%i`, `%u`, `%q`). Each is a separate small ADR if needed.
- ❌ Width / precision / flag suffixes.

## Decision

Same shape as ADR 0202:

- `FormatSpec` gains `Octal` and `GeneralFloat` variants.
- `parse_format_specifiers` matches `'o'` / `'g'`.
- Per-arg validation: both require Number.
- Codegen: `'o'` joins the f2i + i64 push path with `%d` / `%x` / `%X`. `'g'` joins the f64 direct-push path with `%f`.

## Tests

`tests/phase4_string_format_oct_gfloat.rs` (NEW, 2 e2e):

1. `string.format("%o", 8)` → `"10"`.
2. `string.format("%g", 1.5)` → `"1.5"`.

## Test count delta

```
Step 0: 1447 (after 5e5e599)
C3 (impl): 1447 → 1449
```

## References

- [Lua 5.4 §6.4 string.format](https://www.lua.org/manual/5.4/manual.html#pdf-string.format)
- [ADR 0152](0152-string-format.md), [ADR 0202](0202-string-format-hex.md).
