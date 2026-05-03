# 0051. Phase 2.7m: `assert(cond, msg)` with Optional Custom Message

- **Status:** Accepted
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Phase 2.7g (ADR 0030) shipped `assert(cond)` — a 1-arg
predicate that exits with the canned message
`assertion failed!` when `cond` is false. Real Lua's
`assert` is 1-or-2-arg:

```lua
assert(t ~= nil, "table must not be nil at startup")
```

The second arg is the user-provided failure message,
typically built with concat to include context. Without it,
every assert produces the same generic line — useless for
diagnosing real failures.

## Decision

### Variadic-bounded arity (1 ≤ N ≤ 2)

Add a special-case branch alongside Phase 2.8b's `Print`
variadic handling:

```rust
if matches!(builtin, Builtin::Assert) {
    if args.is_empty() || args.len() > 2 {
        return Err(HirError::ArityMismatch { … });
    }
} else if !matches!(builtin, Builtin::Print) {
    let arity = builtin.arity();
    if args.len() != arity { … }
}
```

`Builtin::arity()` keeps reporting `1` for backward
compatibility with the existing call signature; the special
case lifts that to a 1..=2 bound.

### Per-position kind dispatch

The argument-type loop now branches on the arg's index:
position 0 → `Bool`; position 1 → `String`. The check
shape matches the existing `Builtin::Error` String-only
constraint:

```rust
if matches!(builtin, Builtin::Assert) {
    let arg_idx = …;
    let expected = if arg_idx == 0 { Bool } else { String };
    if k != expected { return Err(TypeMismatch { … }); }
}
```

### Codegen: `emit_assert` takes `Option<Value>`

```rust
fn emit_assert<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    cond: Value<'c, 'a>,
    custom_msg: Option<Value<'c, 'a>>,
    types: &Types<'c>,
    loc: Location<'c>,
)
```

The failure-path region now picks the message ptr based on
the option:

```rust
let msg_ptr = match custom_msg {
    Some(v) => v,
    None => emit_addressof(context, &then_blk, "s_assert_failed", types, loc),
};
```

The 2nd arg expression evaluates **unconditionally** in the
outer block (it's an ordinary call argument), and the
`scf.if` failure path consumes the resulting ptr without
re-evaluating. This matches Lua's eager-evaluation
semantics and keeps the codegen simple.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | Bounded-arity branch; per-position kind dispatch    |
| Codegen  | `emit_assert` gains `Option<Value>` param; failure-path picks msg ptr |

The 1-arg `assert(cond)` form is unchanged at every layer —
users without a 2nd arg get the original `assertion failed!`
default.

## TDD Process

1. **Red.** 8 e2e tests covering pass-with-msg, fail-with-
   custom-msg, fail-without-msg regression, msg-as-local,
   msg-as-concat, zero-args rejection, three-args
   rejection, non-String-msg rejection. 4 failed (2-arg
   form unsupported); 4 already passed (regression + the
   bounds rejections via the existing arity check).
2. **Green.** HIR bounded-arity branch + per-position kind
   check; codegen `Option<Value>` plumbing in
   `emit_assert`. All 8 tests pass at 657 (649 + 8).
3. **Refactor.** None warranted — the special-case
   branches are localised and small.

## Alternatives Considered

- **Use the canned default and append the msg as suffix.**
  Pleasant for casual programs but loses Lua semantics:
  `assert(cond, "x")` and `error("x")` should produce the
  same final string in compatible runtimes.
- **Auto-coerce the msg via `tostring`** so `assert(false,
  42)` works. ADR 0026 deliberately limited auto-coerce to
  the concat operator; widening it to assert's 2nd arg
  would set a precedent. Reject — explicit `tostring` is
  one keystroke and matches our project-wide stance.
- **Generalize all builtins to a `min..=max` arity range**
  instead of one special case per variadic builtin. Cleaner
  but premature — we have exactly two non-fixed builtins
  now (`print` variadic, `assert` 1-or-2). When a third
  arrives, extract.

## Consequences

- HIR adds ~25 lines (bounded-arity branch + per-position
  kind dispatch).
- Codegen gains an `Option<Value>` parameter on
  `emit_assert` (~10 lines net).
- 8 new e2e tests; total green at 657.

## Out of Scope

- **Pass-through return value of `assert`'s argument** —
  Lua's `local x = assert(maybeNil, "x")` returns
  `maybeNil` (typed). Our `assert` returns `Bool` (the
  cond value). Generalising requires heterogeneous-return
  builtins.
- **Multi-arg pass-through `assert(a, b, c, …)`** — Lua
  passes the rest of the args through too. Defer.
- **Auto-coerce of msg via tostring** — see Alternatives.
- **Lua's "Table-as-message" form** for structured errors.
  Pending tables.
