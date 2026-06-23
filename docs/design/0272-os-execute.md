# 0272. `os.execute(cmd)` (N7-11)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::OsExecute`; arity `(1, 1)`; param `[String]`; ret `[Bool]`.
- ✅ Codegen: declares libc `int system(const char*)`, extracts raw byte ptr via `emit_string_obj_data`, returns `Bool = (rc == 0)`.
- ❌ Full Lua 5.4 multi-return `(success, exit_kind, code)` — deferred.
- ❌ No-arg form (`os.execute()` returns whether a shell is available) — deferred.

## Tests

2 e2e: `os.execute("true")` → true; `os.execute("false")` → false. 1681 → 1683.

## References

- Lua 5.4 §6.9.
- ADRs 0264-0271 — sibling N7 increments.
