# 0267. `os.tmpname()` (N7-6)

- **Status:** Accepted
- **Date:** 2026-06-23

## Scope (literal)

- ✅ New `Builtin::OsTmpname`; arity `(0, 0)`; ret `[String]`.
- ✅ Codegen: declares `char *tmpnam(char *)`. Calls `tmpnam(NULL)` (single static buffer per POSIX), strlen's the result, wraps via `emit_string_obj_from_bytes` into a fresh GC-tracked String.
- ❌ POSIX deprecated `tmpnam` (race condition between name return and file creation); Lua 5.4 spec keeps the API. A future ADR may swap to `mkstemp` for the security-aware variant.

## Tests

`tests/phase4_n7_os_rename_tmpname.rs` tmpname arm (1 e2e): returns non-empty String. 1668 → 1669.

## References

- Lua 5.4 §6.9.
- ADR 0265 / 0266 — sibling os.* increments.
