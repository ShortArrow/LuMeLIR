# 0085. Phase 2.8e-iter-generic: Generic-for Protocol Parser Sugar

- **Status:** Accepted
- **Date:** 2026-05-07
- **Deciders:** ShortArrow

## Context

Lua 5.4 §3.3.5 generic-for: `for var_list in expr_list do BODY end`. The
expression list resolves to a 3-tuple `(iter, state, ctl)` and each
iteration calls `iter(state, ctl)`, terminating when the first result
is `nil`. ADR 0078 (`ipairs`) and ADR 0080 (`pairs`) shipped restricted
sugars for the two most common cases; the full 3-tuple form remained
parser-rejected as `LIC-2.8e-iter-generic-1`.

ADR 0080 / 0081 / 0082 ship every codegen primitive ADR 0085 needs:
- ADR 0080: ForPairs synthetic-block desugar pattern (the template).
- ADR 0081: `Builtin::Next` + `MultiAssignFromCall(Callee::Builtin)` —
  the `for k, v in next, t, nil` form.
- ADR 0082: `Callee::IndirectDispatch` for TaggedValue-callable iter.

So ADR 0085 is parser + HIR only — codegen needs no new arm.

Codex pre-ADR-0085 review picked Option A (generic-for) over Option C
(NaN diagnostic) and Option B (ADR 0083 closure feasibility spike) and
strongly recommended:

- **No new HIR shape**: synthetic block desugar mirroring ADR 0081's
  ForPairs.
- **Iter / state / ctl pinning** to fresh locals before the while
  loop — re-evaluation hazard if iter is an expression with side
  effects.
- **Reuse `Callee::IndirectDispatch`** for TaggedValue iter.
- **Phase 1 scope: static-compatible iter only** — non-capturing
  user functions, `Builtin::Next`, function aliases. Closure-as-iter
  is filtered out and re-evaluated when ADR 0083 lands the
  env-threading ABI.

## Decision

### AST + Parser

`StmtKind::ForGeneric { names, iter, state, ctl, body }` joins
`ForIpairs` and `ForPairs` as the third for-in variant. `IterMatch`
gains a `Generic { iter, state, ctl }` discriminator.

`parse_for` recognises the 3-tuple form by peeking for a `,` after
the first expression that follows `in`:

- `for k, v in EXPR1, EXPR2, EXPR3 do` → `ForGeneric`.
- `for k, v in ipairs(EXPR) do` → `ForIpairs` (existing).
- `for k, v in pairs(EXPR) do` → `ForPairs` (existing).
- 1-name forms (`for k in ...`) and 3+-name forms remain parser-
  /HIR-rejected. Users can pad with `_` to force the 2-name shape.
- A bare single-expression iter (`for k, v in iter_call() do`) without
  the comma — Lua's "implicit `state = nil, ctl = nil`" — is **not**
  in scope for ADR 0085 Phase 1; the existing `UnsupportedIterator`
  error fires.

### HIR `lower_stmt(StmtKind::ForGeneric)`

Synthetic block desugar (template inherited from ADR 0081 ForPairs):

```text
do
  local __state = STATE              -- pinned in fresh inner scope
  local __ctl   = CTL                -- TaggedValue
  local __iter  = ITER               -- only when iter is not the `next` builtin
  local _broken_N = false
  while true do
    local k, v = __iter(__state, __ctl)   -- MultiAssignFromCall (iter callee)
    if IsNil(k) then _broken_N = true
    else BODY ; __ctl = k end
  end
end
```

The `__iter` local is omitted when iter resolves to `Builtin::Next` —
the builtin dispatch is direct without indirection.

### Iter resolution dispatch

The `MultiAssignFromCall.callee` is computed up-front from the source
iter expression:

| Source iter shape                                 | Callee                                                                                              |
|---------------------------------------------------|-----------------------------------------------------------------------------------------------------|
| `ExprKind::Ident("next")`                         | `Callee::Builtin(Builtin::Next)`                                                                    |
| `HirExprKind::FunctionRef(fid)`                   | `Callee::User(fid)`                                                                                 |
| `HirExprKind::Local(idx)` of `Function(arity=2)` with known FuncId | `Callee::User(fid)`                                                                |
| `HirExprKind::Local(idx)` of `Function(arity=2)` parameter (no FuncId) | rejected — function params return single Number per ADR 0019, can't satisfy 2-result iter ABI |
| `HirExprKind::Local(idx)` of `TaggedValue`        | `Callee::IndirectDispatch { sig, candidates }` — full-sig filter excludes closures-with-upvalues   |

The TaggedValue branch's candidate filter:

```rust
let candidates: Vec<FuncId> = self.functions.iter().enumerate()
    .filter_map(|(i, f)| {
        let pk: Vec<ValueKind> = f.params.iter().map(|p| p.kind).collect();
        (pk == sig.param_kinds && f.ret_kinds == sig.ret_kinds && f.upvalues.is_empty())
            .then_some(FuncId(i))
    })
    .collect();
```

`f.upvalues.is_empty()` is the closure-as-iter filter — when ADR 0083
lands the env-threading ABI, drop the predicate to lift Phase 1's
restriction automatically.

### Iter return-shape constraint

The iter must return 2 values (per Lua spec), and the first must be
**`TaggedValue` or `Nil`** so the loop can receive `nil` as the
termination sentinel:

- Number-only or Bool-only first ret_kind → `TypeMismatch` at HIR.
  Loop would never terminate (no nil reachable in the static type).
- `Nil` static first ret_kind is accepted (the iter immediately
  returns nil, body never runs).
- `TaggedValue` first ret_kind covers the typical case where a user
  function with mixed-shape returns has been widened by ADR 0074.

For non-`next` user-fn iters, this means the iter body needs at least
one `return` (with no values, widened to TaggedValue by ADR 0074) or
explicit `return nil, nil`.

### What stays out

- Closure-with-upvalues iter (`local iter = make_iter(n); for k in
  iter, …` where `make_iter` returns a capturing closure) — rejected
  by the candidate filter and the existing escape backstops (ADR
  0044 / 0071). Will be lifted when ADR 0083 (full closures) lands.
- 1-name forms — Lua spec allows `for k in ...` but ADR 0085 keeps
  the existing 2-name parser shape for simplicity. Users can pad
  with `_`.
- 3+-name forms — same restriction.
- Implicit-state form (`for k, v in iter_call() do`) — requires
  arbitrary expression in iter slot with implicit `state = nil, ctl
  = nil` per Lua spec; out of Phase 1 scope.

## Alternatives Considered

- **New `HirStmtKind::ForGeneric` opaque shape** (parallel to ADR
  0080's pre-refactor ForPairs): rejected. ADR 0081 explicitly walked
  back ForPairs's opaque shape because `MultiAssignFromCall` +
  `Callee::IndirectDispatch` already cover the codegen surface; the
  same logic applies here.
- **Parse-time iter introspection** (recognise `iter, state, ctl` and
  decide call protocol at parse): rejected. Codex CA review §3 was
  explicit — iter resolution belongs in HIR, parser only sees surface
  syntax.

## Consequences

- **`LIC-2.8e-iter-generic-1` → resolved (Phase 1 scope).** Closure-
  as-iter is documented as carry-over to ADR 0083 follow-up rather
  than a separate pending LIC entry.
- **Test totals: 951 → 959 green.** 8 new e2e in
  `tests/phase2_8e_generic_for.rs`: next-builtin form, user-fn iter,
  function-alias iter, break, nested generic-for, immediate-nil
  termination, closure-as-iter rejection backstop, Number-only iter
  rejection backstop.
- **LIC totals: 24 / 0 / 3 → 25 / 0 / 3** (resolved / partial /
  pending).
- **Source LOC**: AST `+10`, parser `+30`, HIR `+200` (4 visitor
  companions + the desugar), tests `+200`.

## Refactor path (ADR 0083 follow-up, future-ADR candidate — Function-kind upvalue support)

When ADR 0083 ships full closures (heap-allocated closure object +
shared upvalue boxes), the closure-as-iter filter in
`lower_stmt(StmtKind::ForGeneric)` becomes one line to delete:

```rust
.filter_map(|(i, f)| {
    let pk: Vec<ValueKind> = f.params.iter().map(|p| p.kind).collect();
    (pk == sig.param_kinds && f.ret_kinds == sig.ret_kinds /* && f.upvalues.is_empty() */)
        .then_some(FuncId(i))
})
```

The `make_iter` factory pattern (`local iter = make_iter(n); for k, v
in iter, state, ctl do …`) automatically starts working — no other
change to ADR 0085's shape is needed.

## Documentation updates

- [x] §1 slot layout — n/a.
- [x] §2 producer / source taxonomy — n/a.
- [x] §3 consumer matrix — n/a.
- [x] §4 LIC consolidation — `iter-generic-1` moved to Resolved with
      a Phase 1 scope note. Totals: **27 entries — 25 resolved / 0
      partial / 2 pending core + 2 pending runtime-diag**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Generic-for protocol" subsection
      describing the synthetic-block desugar and the iter resolution
      dispatch table.
- [x] §7 open questions — `iter-generic-1` removed; ADR 0083 (Full
      closures, deferred) remains #1; closure-as-iter / closure-with-
      upvalues lift will be a future-ADR follow-up (Function-kind upvalue support).
- [x] §8 ADR index — ADR 0085 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative list
(ADR 0068).
