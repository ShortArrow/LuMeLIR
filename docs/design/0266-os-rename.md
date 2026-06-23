# 0266. `os.rename(old, new)` (N7-5)

- **Status:** Accepted
- **Date:** 2026-06-23

## Scope (literal)

- ✅ New `Builtin::OsRename`; arity `(2, 2)`; param `[String, String]`; ret `[Bool]`.
- ✅ Codegen: declares `int rename(const char*, const char*)`, extracts raw bytes from both String args via `emit_string_obj_data`, returns `Bool = (r == 0)`.
- ❌ `(nil, errmsg)` multi-return form deferred.

## Tests

`tests/phase4_n7_os_rename_tmpname.rs` rename arms (2 e2e): success + missing-source false. 1666 → 1668.

## References

- Lua 5.4 §6.9. ADRs 0264 / 0265 sibling.
