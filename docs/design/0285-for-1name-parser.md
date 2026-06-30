# 0285. Generic-for 1-name parser shape (N4-F-2a)

- **Status:** Accepted (parser-only; full end-to-end gated on N4-F-2b/c)
- **Kind:** Architecture Decision
- **Date:** 2026-06-30
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Parser accepts `for x in iter_expr do body end` (1-name generic-for).
- ✅ Desugar at AST level: equivalent to `StmtKind::ForGeneric { names: [var, "_for1_discard"], iter: iter_expr, state: Nil, ctl: Nil, body }`.
- ✅ The synthetic second binding `_for1_discard` is a regular AST identifier — HIR creates a normal Local that the body never references, so DCE / unused-binding warnings will eventually elide it.
- ❌ HIR / codegen support for arbitrary iter — still inherits the existing ForGeneric contract that iter return `(TaggedValue, TaggedValue)`. Single-return iters won't run yet.
- ❌ End-to-end execution of `for word in string.gmatch(s, pat) do ... end` — still gated on N4-F-2c (the gmatch builtin) and possibly an HIR contract loosening (single-return iter).

## Why parser-only

The 1-name form has two callers in practice:

1. `gmatch`-style iterators that return a single value per call (Lua spec).
2. Manual user closures that may return 1 or 2 values.

The existing 2-name ForGeneric HIR lowering hard-codes `iter_ret_kinds.len() != 2 → ArityMismatch`. Relaxing that to accept 1-return is its own session: every downstream codegen path (the loop's nil-check, the variable binding, the multi-assign machinery) currently assumes 2 returns. That work belongs to N4-F-2b alongside the synthetic-function infrastructure.

What this ADR locks in is the **syntactic acceptance** so future sessions can land HIR + closure-source work without parser churn.

## Tests

3 parser-only e2e (`tests/phase4_n4f2a_for_1name_parser.rs`): bare local iter; `f()`-call iter; `t.iter` table-field iter. 1747 → 1750.

## References

- ADR 0085 — generic-for protocol (2-name + 3-tuple form).
- ADR 0283 — N4-F-1 `string.find(s, pat, init)`.
- ADR 0284 — N4-F-2 deferred design.
- Lua 5.4 §3.3.5 — generic for.
- Roadmap 2026-06-27 N4-F.
