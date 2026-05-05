# 0078. Phase 2.8e-iter-ipairs: `for k, v in ipairs(t) do ...` Sugar

- **Status:** Accepted
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

## Context

Lua spec §3.3.5: the **generic for** loop drives iteration via a
3-tuple `(iter, state, control)` where `iter` is a callable
function. `pairs(t)` and `ipairs(t)` are the two standard
iterator factories — `ipairs` for sequential array iteration
stopping at the first nil, `pairs` for the full key/value
protocol. Generic for repeatedly invokes `iter(state, control)`
to obtain the next pair.

LuMeLIR's current state blocks the full-fidelity implementation:

- **ADR 0075** rejects every indirect call through a TaggedValue
  local. The generic-for protocol is exactly that — `iter` is a
  Function value whose static `FuncId` is unknown to the call
  site. Resolving this needs a signature side table or a similar
  ADR-0075 superseder.
- `pairs(t)` additionally requires a hash-bucket-walk protocol
  with awareness of tombstones (ADR 0062), and the plumbing for
  arbitrary key kinds (LIC-2.6a-arr-3, partial).

Codex post-ADR-0077 review made iteration the **#1 priority**
and recommended **Plan C**: implement `ipairs` only as a
parser-level Lua sugar, desugaring to existing HIR primitives
without touching the indirect-call boundary. `pairs` and
generic-for remain LIC-tracked.

## Decision

### `for IDX, VAL in ipairs(TABLE) do BODY end` is a parser sugar

The parser pattern-matches on `for NAME, NAME in EXPR do …`
where `EXPR` is exactly `ipairs(TABLE)` (single argument). Any
other iterator (`pairs(t)`, custom callables, multi-arg
`ipairs(t, k)`) returns `ParseError::UnsupportedIterator`
pointing at the relevant LIC.

```rust
// Parser:
fn unwrap_ipairs_call(expr: Expr) -> Option<Expr> {
    if let ExprKind::Call { callee, args } = expr.kind
        && let ExprKind::Ident(ref name) = callee.kind
        && name == "ipairs"
        && args.len() == 1
    { /* return args[0] */ }
}
```

A new AST variant `StmtKind::ForIpairs { idx_name, val_name,
table, body }` carries the parsed structure; a new
`Keyword::In` keyword is added to the lexer.

### HIR desugars to existing primitives

`lower_stmt(ForIpairs)` produces a `Block` of:

```text
do
  local __t = TABLE                  -- LocalInit (Table-kind)
  local IDX = 1                       -- LocalInit (Number)
  local _broken_N = false             -- LocalInit (Bool)
  while true do
    local VAL = __t[IDX]              -- LocalInit IndexTagged → TaggedValue
    if IsNil(VAL) then
      _broken_N = true                -- exit on first nil
    else
      BODY                            -- guarded with `if not _broken_N then …`
                                      -- per ADR 0015 break-flag semantics
      IDX = IDX + 1
    end
  end
end
```

Key points:
- `IndexTagged` (ADR 0063) widens `VAL` to TaggedValue — the
  body sees the array element through the tagged dispatch chain
  without a trapping read.
- `IsNil` (ADR 0066) is the non-trapping nil probe; first-nil
  termination is exactly the Lua-spec semantics for `ipairs`.
- The `else` branch hosts BODY + `IDX += 1`, ensuring the body
  never runs with `VAL == nil` (Lua: stop, don't expose nil).
- Body is lowered through `lower_scoped_body_no_push` so each
  statement is `if not _broken then STMT end`-wrapped, giving
  `break` mid-body the standard short-circuit semantics
  (ADR 0015).

### Codegen change: zero

All synthesised statements are existing `HirStmtKind` shapes
(`LocalInit`, `Assign`, `While`, `If`, `BinOp`, `IsNil`,
`IndexTagged`). The `emit_stmt` and `emit_expr` arms already
cover them.

### What does not work

- `pairs(t)` (LIC-2.8e-iter-pairs-1): hash bucket walk requires
  exposure of internal hash layout + tombstone state across an
  iteration; deferred until `pairs` becomes worth implementing
  alongside hash key kind expansion.
- Generic for with arbitrary callable iter (LIC-2.8e-iter-
  generic-1): the iterator function is a runtime callable
  whose signature is unknown statically — the same hazard ADR
  0075 closes.
- Single-variable `for k in iter() do`: rejected. `ipairs`
  always yields `(idx, val)`, so two names are required.
- Aliasing `local f = ipairs`: out of scope; `ipairs` is a
  syntactic marker, not a first-class function value in this
  phase.

## Alternatives Considered

- **Plan A (full Lua spec)**: implement `pairs` and `ipairs` as
  real builtins returning iterator functions, plus generic-for
  protocol. Conflicts with ADR 0075's TaggedValue indirect-call
  reject and requires the signature-side-table design first.
  Rejected.
- **Plan B (sugar for both pairs and ipairs)**: same parser
  approach, but also recognise `pairs(t)` and synthesise a
  hash-walk loop. The hash walk needs internal-bucket access
  exposed to HIR/codegen and a stable iteration order across
  resizes — too much new surface for one phase. Defer `pairs`.
- **Plan D (non-Lua `for_each_array(t, fn)` builtin)**: spec
  violation, rejected on principle.

## Consequences

- **`LIC-2.8e-iter-ipairs-1` → resolved.** The most common
  Lua iteration idiom (`for i, v in ipairs(t) do …`) compiles.
- **New `LIC-2.8e-iter-pairs-1`** (pending): `pairs(t)` hash
  iteration.
- **New `LIC-2.8e-iter-generic-1`** (pending): generic-for
  protocol with arbitrary callable iter.
- **Test totals: 898 → 908 green.** 10 new e2e tests in
  `tests/phase2_8e_ipairs.rs` covering basic / empty / single-
  element / nil-stop / sum / concat / break / nested cases plus
  parser-level reject backstops for `pairs` and arbitrary iter.
- **Lexer**: +1 `Keyword::In` variant (~5 LOC).
- **AST**: +1 `StmtKind::ForIpairs` variant (~10 LOC).
- **Parser**: ~75 LOC (`parse_for` branch on `,` for sugar form,
  `unwrap_ipairs_call` helper, `ParseError::UnsupportedIterator`).
- **HIR**: ~150 LOC (`lower_stmt` ForIpairs arm desugar +
  recurse-arm for body-walks like `ast_max_return_arity` and
  `infer_param_kinds`).
- **Codegen**: 0 LOC (everything routes through existing arms).
- **`break` in body** works via the existing ADR 0015 mechanism:
  `lower_scoped_body_no_push` wraps each user statement with
  `if not _broken then STMT end`, so a mid-body break skips the
  remainder.

## Documentation updates

- [x] §1 slot layout — n/a (no new tags or slot shapes).
- [x] §2 producer / source taxonomy — n/a (the tagged read
      inside the loop is the existing `IndexTagged` path).
- [x] §3 consumer coverage matrix — n/a (loop-body consumers
      see TaggedValue VAL via the existing `Local(TaggedValue)`
      column; no new dispatch).
- [x] §4 LIC consolidation — `iter-ipairs-1` resolved;
      `iter-pairs-1` and `iter-generic-1` added pending.
      Totals: **19 resolved / 1 partial / 3 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Iteration desugar" subsection
      showing the AST → HIR equivalence (`for IDX, VAL in
      ipairs(t) do …` → `do local __t … while true do …
      end end`).
- [x] §7 open questions — `iteration` removed; `hash key kinds
      expansion` promoted to #1.
- [x] §8 ADR index — ADR 0078 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
