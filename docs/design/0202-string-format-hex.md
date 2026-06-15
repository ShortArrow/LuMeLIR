# 0202. `string.format` Hex Specs — `%x` / `%X`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Bucket B real gap: ADR 0152 introduced `string.format` with the minimum set `%d` / `%f` / `%s` / `%%`. Hex specs `%x` / `%X` are common idioms (debug printing addresses, masks, etc.). Adding them is a straightforward extension of the existing dispatch.

## Scope (literal)

- ✅ `FormatSpec::HexLower` / `HexUpper` variants.
- ✅ `parse_format_specifiers` accepts `%x` / `%X`.
- ✅ Per-arg validation: hex specs require Number argument (same as Decimal).
- ✅ Codegen rewrites `%x` / `%X` → `%llx` / `%llX` in the printf format buffer; args go through `f2i` (i64) same as `%d`.
- ❌ Width / precision / flag suffixes (`%08x`, `%.4x`, etc.). Same scope ceiling as ADR 0152.
- ❌ Octal `%o`, scientific `%e/%g`, char `%c`, unsigned `%u`. Each is a separate small ADR if needed.

## Decision

### `src/hir/mod.rs`

`FormatSpec` enum gains `HexLower` and `HexUpper`. `parse_format_specifiers` matches `'x'` and `'X'`. Validation arm joins Decimal/Float for Number-kind requirement.

### `src/codegen/emit.rs`

`StringFormat` emit arm's printf-format builder matches `b'x'` / `b'X'` → emit `%llx` / `%llX`. The per-arg loop joins them with `b'd'` in the f2i path.

### Tests

`tests/phase4_string_format_hex.rs` (NEW, 2 e2e):

1. `string.format("%x", 255)` → `"ff"`.
2. `string.format("%X", 255)` → `"FF"`.

## Test count delta

```
Step 0: 1445 (after 5216dd6)
C3 (impl): 1445 → 1447
```

## References

- [Lua 5.4 §6.4 string.format](https://www.lua.org/manual/5.4/manual.html#pdf-string.format)
- [ADR 0152](0152-string-format.md) — `string.format` MVP scope.
