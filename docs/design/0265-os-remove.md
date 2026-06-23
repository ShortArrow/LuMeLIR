# 0265. `os.remove(filename)` (N7-4)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Fourth N7 sub-ADR. Per Lua 5.4 §6.9, `os.remove(filename)` deletes the named file; returns true on success, `(nil, errmsg)` on failure.

## Scope (literal)

- ✅ New `Builtin::OsRemove` HIR variant. Arity `(1, 1)`. Param kinds `[String]`. Ret kind `[Bool]`.
- ✅ Codegen: declares `int remove(const char*)` at module init, extracts the raw byte payload from the boxed String via `emit_string_obj_data`, calls `remove`, returns `Bool = (r == 0)`.
- ❌ **`(nil, errmsg)` on failure** — today returns a plain Bool. Multi-return + errno-to-string mapping is a focused follow-up.

## Tests

`tests/phase4_n7_os_remove.rs` (NEW, 2 e2e): existing file → true + filesystem effect; missing file → false.

## Test count delta

```
Step 0:  1664 (after ADR 0264)
N7-4:    1664 → 1666
```

## References

- [Lua 5.4 §6.9](https://www.lua.org/manual/5.4/manual.html#6.9) — os library.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N7.
