# 0298. Vararg spread — pack forwarding + `{...}` alias (F1-C-step3-partial)

- **Status:** Accepted (partial; dynamic-arity contexts remain deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ **Pack forwarding**: `inner(...)` where `...` is the *sole* extra argument to a vararg callee passes the caller's own `_va_pack` by reference. All extras flow through — full multi-value semantics for the pass-through shape (the dominant real-world use: wrapper functions).
- ✅ **`{...}` alias**: a table constructor whose only field is positional `...` lowers to `Local(_va_pack)` — an alias of the pack, not a copy. Enables `#{...}` (count idiom) and `local t = {...}` element access.
- ✅ **Empty-pack nil confirmed**: `f()` then `...` reads `_va_pack[1]` through the existing Index nil-on-missing path → `nil`, matching Lua. The OOB-trap concern noted in ADR 0297 was wrong — pinned by test.
- ❌ **`{...}` copy semantics** — the alias shares mutation with later `...` reads inside the same call. Lua spec requires a fresh table. Deviation documented; a copying desugar (loop over pack via `pending_pre_stmts` hoisting) replaces the alias in step3-final.
- ❌ **Mixed constructor `{a, ...}`** — falls through to the ordinary path where `...` contributes only its first value. Deviation (Lua spreads all at tail position).
- ❌ **`return ...`** — dynamic-arity return is fundamentally incompatible with the current fixed-arity multi-return ABI (`ret_kinds: Vec<ValueKind>` is compile-time). Blocked until a dynamic-return ABI exists; likely lands with the coroutine work (N5) that needs similar machinery.
- ❌ **`f(a, ...)` prefix spread** — same dynamic-arity blocker on the argument side. Only the sole-`...` forwarding shape is wired.
- ❌ **`select` / `table.pack` / `table.unpack`** — next N7 increments; `table.pack(...)` can now desugar to `{...}` + `n` field once copy semantics land.

## Semantics table (updated from ADR 0295/0297)

| Program | Lua 5.4 | LuMeLIR after this ADR |
|---|---|---|
| `f(42)` → `local t = ...` | `42` | `42` ✅ |
| `f()` → `local t = ...` | `nil` | `nil` ✅ |
| `count(1,2,3)` → `#{...}` | `3` | `3` ✅ |
| wrapper `fwd(...)` → `inner(...)` sees all | all | all ✅ |
| `{...}` then mutate, re-read `...` | independent | aliased ⚠️ deviation |
| `{a, ...}` | spreads tail | first value only ⚠️ |
| `return ...` | spreads | HIR/codegen unsupported ❌ |

## Tests

+3 e2e in `tests/phase4_f1c_vararg_codegen_stub.rs` (7 total): empty-pack nil pinned; `#{...}` = 3; forwarding passes all 4; pack elements readable by index. Full suite 1807.

## References

- ADR 0296 — step2/3 design.
- ADR 0297 — step2 Table-pack ABI.
- Lua 5.4 §3.4.11.
