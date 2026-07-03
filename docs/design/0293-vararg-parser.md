# 0293. Vararg `...` parser shape (F1-A)

- **Status:** Accepted (parser + AST only; HIR / codegen wiring deferred to F1-B / F1-C)
- **Kind:** Architecture Decision
- **Date:** 2026-07-03
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Lexer: new `TokenKind::DotDotDot` token. The `..` scanner branch peeks a third `.` to decide between `DotDot` and `DotDotDot`; regression covered.
- ✅ AST: `ExprKind::FunctionExpr` / `StmtKind::FunctionDef` / `StmtKind::MethodDef` gain `is_vararg: bool`. New `ExprKind::Vararg` for expression-position `...`.
- ✅ Parser: `parse_function_signature_and_body` accepts a trailing `...` in the parameter list and sets `is_vararg = true`. Expression parser (`parse_primary`) recognises `...` and produces `ExprKind::Vararg`.
- ✅ HIR: intentionally rejects both shapes with new `HirError::VarargUnsupported` so downstream stages fail loudly and the wiring cannot silently corrupt.
- ❌ HIR representation of variadic pack — F1-B.
- ❌ Codegen call-site expansion (`f(...)`, `{...}`, multi-return propagation) — F1-C.
- ❌ Downstream builtins `select`, `table.pack`, `table.unpack` — depend on F1-B/C.
- ❌ Tightening `f(..., x)` to a parse error — deferred. Currently the parser breaks out of the param loop after `...`, and a following comma yields an unexpected-token error at close-paren time; the regression test covers "does not panic" but doesn't pin a precise error message.

## Design notes

The three-way choice for representing varargs at HIR:

1. **Packed Table** (per-call materialisation) — every `...` inside the body reads from a Table upvalue holding the variadic args. Simple but always allocates.
2. **SSA multi-value** — extend the multi-return machinery so `...` behaves like a multi-return call result: propagates through call, truncates to first when used as a single value.
3. **Hybrid** — SSA for the fast path (direct passthrough as call arg), Table materialisation only when `select` / `table.pack` demand a first-class handle.

F1-B will pick between (2) and (3). This ADR does not lock the choice; it just gets the parser + AST out of the way.

## Tests

9 parser-only e2e (`tests/phase4_f1a_vararg_parser.rs`): `...` in signature; with named params first; anonymous fn; expression position; call argument; table constructor; method definition; `f(..., x)` doesn't panic; `..` still concat. 1786 → 1795.

## References

- Lua 5.4 §3.4.11 — `...`.
- ADR 0017 — anonymous FunctionExpr origin.
- ADR 0092 — MethodDef.
- Roadmap 2026-07-03 — F1-A first step of the varargs chain.
