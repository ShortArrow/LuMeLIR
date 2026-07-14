# 0314. `string.gmatch` via lowering-time closure synthesis (N4-F-2b/2c)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-14
- **Deciders:** ShortArrow

## Context

ADR 0284 decomposed the gmatch closure form into N4-F-2a (1-name
generic-for, landed as ADR 0285), N4-F-2b (synthetic-function
infrastructure), and N4-F-2c (the builtin). 2b and 2c land together
here: the infrastructure's only consumer is gmatch, and a test-only
synthetic builtin would have been pure ad-hoc surface.

## Decision — desugar by reparse, no new Builtin

`string.gmatch(s, pat)` never becomes a `Builtin`. It is desugared
entirely inside `lower_gmatch_iterator` (`src/hir/mod.rs`):

1. Three synthetic locals via `pending_pre_stmts`:
   `__gmatch_s_N` (String), `__gmatch_p_N` (String),
   `__gmatch_pos_N` (Number, init 1).
2. The iterator body is a **Lua snippet** referencing those locals by
   name, parsed with the ordinary parser and wrapped in a synthetic
   `ExprKind::FunctionExpr` AST node:

   ```lua
   local s0, e0 = string.find(<s>, <p>, <pos>)
   if s0 == nil then return nil, nil end
   if e0 < s0 then <pos> = s0 + 1 else <pos> = e0 + 1 end
   return string.sub(<s>, s0, e0), nil
   ```

3. `lower_expr` on that node runs the **standard `FunctionExpr`
   pipeline** — capture analysis resolves the three locals into heap
   upvalue boxes (ADR 0083), so per-call state mutation (`pos`)
   persists across iterator invocations with zero new closure
   machinery. The `e0 < s0` branch is the Lua-spec empty-match
   advance.

Rejected alternatives: builtin-returned closure value (needs a
Function-kind `ret_kinds` consumer arm in codegen that doesn't exist),
codegen-level closure synthesis (bypasses capture analysis), and a
`{s, pat, pos}` state table (heterogeneous table reads reach
`string.find`/`string.sub` as TaggedValue args, whose codegen arms
read the Number payload without narrowing — the String `s` arg then
feeds a raw f64 into the length load).

## The Nil-param call ABI

The generic-for protocol calls `iter(state, ctl)`; gmatch ignores
both. The desugar's `ctl` local is always TaggedValue and holds
`nil`, then each produced match (a String) — `emit_expr` on such a
local is a Number-payload read guarded by `s_table_type_mismatch`,
so it cannot be passed through as an argument. Resolution: the
synthesized params are patched to `ValueKind::Nil`, and
`emit_multi_assign_from_user_call` gains a **Nil-param bypass** —
a Nil-kind param can only ever observe `nil`, so the caller emits
the `i1 0` constant directly and never evaluates the argument.

## Constraints recorded

- `s`/`pat` must be statically String; TaggedValue sources are
  rejected at HIR (`BuiltinArgKindMismatch`) because captured
  TaggedValue upvalue boxes are still `unimplemented` (ADR 0083).
- The iterator's `ret_kinds` land as `[TaggedValue, Nil]` — the
  generic-for gate accepts any nil-terminable first slot.
- Explicit-state forms (`for x in string.gmatch(..), s0, c0`) are not
  meaningful for gmatch and keep whatever kinds the user wrote; only
  the protocol `nil, nil` call shape is exercised.

## Consequences

`lower_gmatch_iterator` doubles as the synthesis precedent: any
future builtin needing a closure result (e.g. `pairs` over
metatables, `io.lines`) can follow the same reparse-and-lower shape.
