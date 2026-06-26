# 0276. `os.setlocale()` no-arg query (N7-15)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-26
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::OsSetlocale`; arity `(0, 0)`; ret `[String]`.
- ✅ Codegen: `setlocale(LC_ALL=6, NULL)` (glibc Linux value, matches current x86_64-linux target) → `char*`; wrapped via `strlen` + `emit_string_obj_from_bytes` — same shape as `os.tmpname` (ADR 0267).
- ❌ 1-arg setter form `os.setlocale(locale)` — deferred.
- ❌ 2-arg `os.setlocale(locale, category)` (category dispatch) — deferred.
- ❌ Nil-on-failure return — POSIX guarantees LC_ALL query is non-NULL after process init, so the no-arg form returns String unconditionally. Setter form will need TaggedValue.
- ❌ Cross-platform LC_ALL constant (macOS = 0, Windows = 0). Tracked alongside N9 multi-target CI.

## Tests

2 e2e (`tests/phase4_n7_os_setlocale.rs`): non-empty string returned; default startup locale is `"C"`. 1692 → 1694.

## References

- Lua 5.4 §6.9.
- ADR 0267 — `os.tmpname` libc-ptr-wrap precedent.
- ADRs 0262-0275 — sibling N7 increments.
