# 0201. `string.reverse(s)` — Byte-Wise Reversal

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Bucket B of [leftover-roadmap.md](../notes/leftover-roadmap.md) lists `string.reverse` as a real gap. Lua 5.4 §6.4 documents it as byte-wise reversal: `string.reverse("abc")` returns `"cba"`. Currently LuMeLIR's `string` namespace dispatches via `Builtin::from_namespace_method`, but there is no `Builtin::StringReverse` variant.

## Scope (literal)

- ✅ New `Builtin::StringReverse` variant.
- ✅ `Builtin::string_from_method("reverse")` returns it.
- ✅ Arity `(1, 1)`; param Number/String reject is via `param_kinds_for_arity` slice.
- ✅ Codegen emits malloc + reverse-memcpy (byte-wise) + NUL terminator.
- ✅ 1 e2e: `print(string.reverse("abc"))` → `"cba"`.
- ❌ UTF-8-aware reverse. Lua spec is byte-wise; matches us.

## Decision

### `src/hir/ir.rs`

```rust
pub enum Builtin {
    // ...
    StringReverse,
}
```

`string_from_method` adds `"reverse" => Some(Builtin::StringReverse)`. `arity` returns `(1, 1)`, `name` returns `"string.reverse"`, `ret_kinds` returns `&[ValueKind::String]`, `param_kinds_for_arity` returns `&[ValueKind::String]`.

### `src/hir/mod.rs::infer_kind`

`Callee::Builtin(Builtin::StringReverse) => ValueKind::String` joins the `string.upper/lower/sub/rep` group.

### `src/codegen/emit.rs`

New emit arm next to `StringUpper`/`StringLower`. Allocates a string-object of length `len(s)`, iterates `i` in `0..len`, copies `s.data[len-1-i]` into `dst.data[i]`. Reuses `emit_string_obj_len` / `emit_string_obj_data` / `emit_string_obj_alloc` / `emit_string_obj_finalize_nul`.

### Tests

`tests/phase4_string_reverse.rs` (NEW, 2 e2e):

1. `print(string.reverse("abc"))` → `"cba"`.
2. `print(string.reverse(""))` → `""` (empty in, empty out).

## Test count delta

```
Step 0: 1443 (after 731e8e5)
C2 (2 Red Day 0): 1443 → 1443
C3 (impl): 1443 → 1445
```

## References

- [Lua 5.4 §6.4 string.reverse](https://www.lua.org/manual/5.4/manual.html#pdf-string.reverse)
- [ADR 0103](0103-stdlib-string-begin.md) — namespace dispatch pattern reused.
