# 0299. `select` + `table.pack` over the vararg pack (F1-D partial)

- **Status:** Accepted (vararg-pack forms; general `select(n, a, b, c)` deferred)
- **Kind:** Architecture Decision
- **Date:** 2026-07-05
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `select("#", ...)` desugars at HIR to `#_va_pack` (UnaryOp Len over the pack Local). Extras count, exact.
- ✅ `select(n, ...)` desugars to `_va_pack[n]`. Single-value position: returns the n-th extra (Lua's "all values from n onward" multi-value tail truncates to the first — the standard single-position deviation, same as `string.find` ADR 0228).
- ✅ Past-the-end `n` yields nil via the Index nil-on-missing path.
- ✅ Works with declared params first: `function f(a, b, ...)` — extras split happens at the call site (ADR 0297), so `select` sees only true extras.
- ✅ `table.pack(...)` desugars to the pack alias + a hoisted `pack.n = #pack` IndexAssign (via `pending_pre_stmts`). Alias — not a copy — same documented deviation as `{...}` (ADR 0298).
- ❌ General literal-args form `select(2, "a", "b", "c")` without `...` — not wired; `select` outside the `(_, ...)` shape still resolves to UndefinedName. Follow-up N7 increment.
- ❌ Negative `n` (select from the end) — deferred.
- ❌ Multi-value tail (`select(n, ...)` returning all from n) — blocked on dynamic-arity ABI (same blocker as `return ...`, ADR 0298).
- ❌ `table.unpack(t)` — return-spread is the same dynamic-arity blocker. The full F1 chain closes when that ABI lands (likely alongside N5 coroutines).

## Implementation

Single pattern-match at the top of `lower_call`: `Call(Ident("select"), [first, Vararg])` inside a vararg function. `"#"` string literal → Len; anything else → lower first arg as index. No `Builtin::Select` enum variant — the desugar never reaches builtin dispatch, keeping the arity/kind plumbing untouched (zero ripple).

## Tests

7 e2e (`tests/phase4_f1d_select.rs`): select count 3; count 0 (no extras); n-th extra; past-end nil; declared-params-first split; `table.pack(...).n` = 3 with element access; empty pack `.n` = 0. Suite 1807 → 1814.

## References

- ADR 0297 — pack ABI this reads from.
- ADR 0298 — sibling spread shapes.
- ADR 0228 — single-position truncation precedent.
- Lua 5.4 §6.1 — `select`.
