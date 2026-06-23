# 0271. `os.date()` no-arg form (N7-10)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::OsDate`; arity `(0, 0)`; ret `[String]`.
- ✅ Codegen: stash `time(NULL)` result in an `llvm.alloca` time_t slot, pass slot ptr to libc `ctime`, then wrap `strlen(buf) - 1` bytes (trim the trailing `'\n'` ctime appends) via `emit_string_obj_from_bytes`.
- ✅ libc `ctime(const time_t*) -> char*` declared at module init.
- ❌ **Format string** (`os.date("%Y-%m-%d", ...)`) — needs `strftime` + buffer-size estimation. Deferred.
- ❌ **`time` arg** — needs the same `strftime` plumbing as above.

## Tests

3 e2e: `type(os.date()) == "string"`, non-empty, trailing `'\n'` stripped.

## References

- Lua 5.4 §6.9.
- ADRs 0264-0270 — sibling N7 increments.
