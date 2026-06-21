# 0242. `os.*` Namespace MVP — `time` / `clock` / `getenv`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third (closing) M11 sub-ADR. Lua 5.4 §6.9 `os` library is the last "core stdlib namespace" missing from LuMeLIR's catalog. This ADR lands the most-used three: `os.time()` (Unix epoch), `os.clock()` (CPU time), `os.getenv(name)` (env vars). All three map directly to libc / POSIX externs.

## Scope (literal)

- ✅ New `os` namespace dispatched by `Builtin::from_namespace_method` (sibling of `io`, `string`, `table`, `rust`).
- ✅ Three new HIR `Builtin` variants: `OsTime`, `OsClock`, `OsGetenv`.
- ✅ `OsTime`: arity 0, returns Number (Unix epoch seconds as f64); calls libc `time(NULL)` → i64 → sitofp.
- ✅ `OsClock`: arity 0, returns Number (CPU seconds as f64); calls libc `clock()` → i64 → sitofp → divide by `CLOCKS_PER_SEC` (1_000_000 per POSIX, hardcoded as compile-time constant).
- ✅ `OsGetenv`: arity 1 (String arg), returns TaggedValue (String-or-Nil); calls libc `getenv(name+8)` — the `+8` skips the boxed-string-object i64 length header (ADR 0112) to pass the raw C-string to libc. Returns String on hit (boxed via `emit_string_obj_from_bytes`), Nil on miss.
- ❌ `os.date` / `os.difftime`. Future M11-stretch.
- ❌ `os.exit` / `os.remove` / `os.rename` / `os.tmpname`. Future M11-stretch.
- ❌ `os.setlocale` / `os.execute`. Future.
- ❌ Subtype-aware Integer return from `os.time` (Lua spec says Integer). The result is currently Number subtype Unknown; `math.type(os.time())` returns `"nil"`. Future M8-extended ADR can mark the result as Integer subtype.
- ❌ Multi-byte / encoding-aware env var values. libc `getenv` returns the raw bytes; we wrap them as a boxed string.

## Decision

### Why `getenv(name+8)`

ADR 0112 boxed-string-object: layout is `[i64 len_le | data[len] | NUL]`. The user-visible String ptr points at the start of the layout (offset 0 is the length header); the actual NUL-terminated C-string starts at offset 8. libc `getenv` expects a C-string, so we pass `name + 8`.

### Why `CLOCKS_PER_SEC` hardcoded to 1_000_000

POSIX requires `CLOCKS_PER_SEC == 1000000`. ISO C says "CLOCKS_PER_SEC shall be implementation-defined" but POSIX pins it. Hardcoding lets the divide be a compile-time constant; if a future non-POSIX target needs flexibility, the constant can move into a libc-extern symbol with a per-platform value.

### `os.getenv` Nil-on-miss shape

Mirrors `io.read` (ADR 0119) and `math.tointeger` (ADR 0211): the result is a TaggedValue tmp slot holding either a String payload or a Nil tag. The non-trapping `if Local(TaggedValue) then ... end` path (ADR 0228) covers `if v then ... end` over the getenv result.

## Tests

`tests/phase4_m11c_os.rs` (NEW, 5 e2e):

1. `os.time() > 0` → `"positive"`.
2. `os.clock() >= 0` → `"ok"`.
3. `os.getenv("LUMELIR_M11C_HIT")` with env set → the value string.
4. `os.getenv("DEFINITELY_NOT_SET_42")` → `"nil"`.
5. `if os.getenv(name) then ... end` matched path → `"set"`.

A new helper `run_ok_env(src, output_name, envs)` lets the test harness set env vars before invoking the compiled binary.

## Test count delta

```
Step 0:  1594 (after ADR 0241)
C3 (impl + 5 e2e): 1594 → 1599
```

## M11 milestone close

ADRs 0240 + 0241 + 0242 land M11 at 3/3-5 minimum-viable close. The math / table / io / os / utf8 / debug expansion items remaining defer to M11-stretch:

- `math.fmod` / `math.modf` / `math.random` / `math.randomseed`.
- `table.pack` / `table.unpack` / `table.sort` / `table.move`.
- `io.open` / `io.close` / `io.lines` (depend on userdata for file handles).
- `os.date` / `os.exit` / `os.remove` / `os.rename`.
- `utf8.char` / `utf8.codepoint` / `utf8.len` / `utf8.offset`.
- `debug.traceback` / `debug.getinfo` / `debug.getlocal` (subset).
- Basic stdlib `select` / `load` / `loadfile` / `dofile`.
- `string.format` width / precision / `%q` (sibling polish).

## References

- [ADR 0103](0103-phase2-7q-stdlib-string.md) — namespace dispatch precedent.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string layout consumed by `getenv`.
- [ADR 0119](0119-phase2-7x-stdlib-io-read.md) — TaggedValue-with-Nil-fallback shape reused.
- [ADR 0228](0228-string-find-plain.md) — non-trapping `if Local(TaggedValue)` cond that lights up `if os.getenv(name) then ... end`.
- [Lua 5.4 §6.9](https://www.lua.org/manual/5.4/manual.html#6.9) — os library spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M11 milestone close.
