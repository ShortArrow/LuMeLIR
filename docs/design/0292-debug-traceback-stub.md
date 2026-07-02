# 0292. `debug.traceback()` stub + opens debug.* namespace (N7-21)

- **Status:** Accepted (stub only — returns empty String)
- **Kind:** Architecture Decision
- **Date:** 2026-07-02
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Opens the `debug.*` namespace via `Builtin::debug_from_method` and adds a `from_namespace_method("debug", ...)` arm.
- ✅ New `Builtin::DebugTraceback`; arity `(0, 0)`; ret `[String]`. Codegen returns `emit_empty_string()`.
- ❌ Real stack walking (frame list, `local` names, source lines) — deferred until libunwind wiring or per-frame metadata tables are added.
- ❌ 1-/2-/3-arg forms `debug.traceback([msg[, level]])` — deferred.
- ❌ Other `debug.*` (getinfo / getlocal / setlocal / gethook / sethook / getregistry) — deferred, this ADR only reserves the namespace so callers don't ParseError-fail.

## Why a stub

Real Lua programs and libraries frequently guard error paths with `debug.traceback()` as the second arg to `xpcall`. A stub keeps those programs compilable and lets them proceed with an empty String traceback until real walking lands.

## Tests

3 e2e (`tests/phase4_n7_21_debug_traceback.rs`): `#debug.traceback() == 0`; `type(debug.traceback()) == "string"`; call runs without crash. 1783 → 1786.

## References

- Lua 5.4 §6.10 — `debug` library.
- Roadmap 2026-07-02 — layer-based sizing (runtime-free codegen-arm stub).
