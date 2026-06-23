# 0264. `os.exit([code])` (N7-3)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Third N7 sub-ADR. Per Lua 5.4 §6.9 `os.exit([code [, close]])` calls libc `exit(int)`. Default code is `0`; the second arg controls whether `close` runs library closers (out of scope here, deferred).

## Scope (literal)

- ✅ New `Builtin::OsExit` HIR variant; lookup via `os_from_method`. Arity `(0, 1)`. Param kinds `[Number]` (optional). Ret kind placeholder `Number` (the call diverges).
- ✅ Codegen: `arith.fptosi(code → i32)` then `llvm.call @exit`. The exit `void` libc decl was already registered for trap paths; we reuse it. After the call a placeholder `arith.constant 0.0 : f64` is yielded so the surrounding expression-typing stays well-formed (the value is never observable).
- ✅ Arity 0 substitutes `0.0` as the default code.
- ❌ **Boolean code form** — `os.exit(true)` (= 0), `os.exit(false)` (= 1). Today's HIR forces `Number`.
- ❌ **`close` flag** — Lua 5.4 spec second arg toggles file-handle closing. Deferred until io.open lands.

## Tests

`tests/phase4_n7_os_exit.rs` (NEW, 3 e2e): default `0`, explicit `42`, divergence-after-statement.

## Test count delta

```
Step 0:  1661 (after ADR 0263)
N7-3:    1661 → 1664
```

## References

- [Lua 5.4 §6.9](https://www.lua.org/manual/5.4/manual.html#6.9) — os library.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N7.
