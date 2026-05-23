# 0049. Phase 2.1a: Multi-Target Reassignment `a, b = …`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Phase 2.5d (ADR 0021) added `local a, b = call()` and the
parallel form `local a, b = 1, 2`. The non-`local` analogue —
plain reassignment — was never added:

```lua
local a = 1
local b = 2
a, b = b, a   -- Lua's idiomatic swap; previously a parse error
```

The parser routes `Ident, Equals` to `parse_assign`. Anything
else after the first ident falls through to expression
parsing, which then errors on the comma. The user sees:

```
parse error: unexpected token Comma
  | a, b = b, a
  |  ^
```

Lua's parallel-evaluation semantic — every RHS evaluated
before any LHS is written — makes the swap work without
explicit temporaries. Without that, simple ports of
real-world Lua code (e.g. anything with sort/swap) hit the
wall immediately.

## Decision

### Parser dispatch widens to multi-name lookahead

```rust
TokenKind::Ident(_)
    if matches!(self.peek_kind_at(1), Some(TokenKind::Comma))
        && self.is_multi_assign_lookahead() =>
{
    self.parse_multi_assign()
}
```

`is_multi_assign_lookahead` walks forward through alternating
`Ident`, `Comma` slots and returns `true` once it sees
`Equals` after at least one comma. Anything else
(`Ident,` followed by a non-`Ident`, EOF before `=`, etc.)
returns `false` and the dispatcher falls back to expression
parsing — which gives the original "unexpected token Comma"
diagnostic. A 32-name sanity bound prevents pathological
inputs from making the lookahead arbitrarily long.

### `StmtKind::AssignMulti` mirrors `LocalMulti`

```rust
AssignMulti {
    names: Vec<String>,
    values: Vec<Expr>,
}
```

Same shape as `LocalMulti` so HIR's pre-scan / param-kind-
inference visit logic shares its arm via `|`-pattern. No
new visit surface.

### HIR lowers in two stages

Stage 1 evaluates every RHS into a fresh temp local, *before*
any target is written:

```rust
let mut tmp_ids = Vec::with_capacity(lowered.len());
for v in &lowered {
    let kind = infer_kind(v, ...);
    let id = self.declare_local(format!("_multi_tmp_{}", ...), kind);
    tmp_ids.push(id);
    block_stmts.push(LocalInit { id, value: v.clone() });
}
```

Stage 2 stores each temp into the matching target:

```rust
for (name, tmp_id) in names.iter().zip(tmp_ids.iter()) {
    let dst_id = match self.resolve(name) {
        Some(id) => { /* kind-check; reject readonly */ id }
        None => { /* auto-declare per ADR 0048, or reject inside fn */ }
    };
    block_stmts.push(Assign { id: dst_id, value: Local(tmp_id) });
}
```

The temp-then-assign sequence delivers Lua's parallel-
evaluation semantic without any new HIR shape: it's just
existing `LocalInit` and `Assign` nodes wrapped in a
`Block`. Codegen needs no change.

### Arity rule

`names.len() == values.len()` is required. Multi-result
call expansion (`a, b = call()`, where one Call returns
two values) is `LocalMulti` / 2.5d territory and is
deliberately **not** added here — the Phase 2.1a scope is
limited to parallel value lists.

### Auto-declare inheritance from ADR 0048

When a target name doesn't resolve and we're at chunk top
level, auto-declare per ADR 0048 (chunk-scope insert,
`scopes[0]`). Inside a function body, the unresolved-name
path errors with `UndefinedName`, matching the single-
target rule.

### CA invariants preserved

| Layer    | Change                                                     |
|----------|------------------------------------------------------------|
| Lexer    | None                                                       |
| Parser   | `parse_stmt` dispatch arm + `is_multi_assign_lookahead` + `parse_multi_assign` |
| AST      | `StmtKind::AssignMulti { names, values }`                  |
| HIR      | `lower_assign_multi` helper; pre-scan / inference visits share arms with `LocalMulti` |
| Codegen  | None — emits via existing `LocalInit` + `Assign` shapes    |

## TDD Process

1. **Red.** 9 e2e tests covering parallel two/three-target
   assignment, swap, three-way rotation, globals as targets,
   String swap, arity-mismatch rejection, kind-mismatch
   rejection, function-body unresolved error, and a
   LocalMulti regression. 8 failed at parse time; 1 (the
   regression) passed.
2. **Green.** Parser dispatch + new helpers + AST variant +
   HIR `lower_assign_multi`. The pre-scan / param-kind-
   inference visits learned the `AssignMulti` arm via
   `|`-pattern next to `LocalMulti`. After fixing one
   non-exhaustive match in the parser's test-helper
   `strip_span_stmt`, all 9 tests pass at 642 (633 + 9).
3. **Refactor.** None warranted. The HIR helper duplicates
   the auto-declare logic from `lower_stmt::Assign`'s 2.0a
   arm; rule of three not yet met (single-target Assign +
   AssignMulti = 2 sites). If a future phase adds another
   write site, extract a shared resolver.

## Alternatives Considered

- **Allow `a, b = call()`** in this phase by sharing
  `MultiAssignFromCall` with `LocalMulti`. Doable but
  introduces a second arity rule (1 value spreads to N
  targets) which complicates the kind-check loop. Defer to
  a follow-up that uses the existing `lower_local_multi`
  Call path.
- **Lua's silent-truncate / nil-pad semantics** for arity
  mismatches. We've consistently chosen explicit errors over
  silent semantic guesses (single-kind slots, explicit
  `tostring`/`tonumber`). Rejecting matches that pattern.
- **Generic Pratt-style lvalue parsing.** Would let the
  parser handle indexed targets (`t[i] = …`) when tables
  arrive. Premature — current scope is bare names.

## Consequences

- Parser + AST + HIR adds ~110 lines; codegen unchanged.
- 9 new e2e tests; total green at 642.
- The Lua swap idiom now works.

## Out of Scope

- **Multi-result Call as RHS** (`a, b = call()`). Defer.
- **Indexed-LHS targets** (`t[i], t[j] = a, b`). Pending
  tables (Phase 2.6).
- **Mixed-arity declarations** (`a, b = 1` truncating to
  `a = 1, b = nil` or rejecting at parser level). Currently
  rejected as ArityMismatch in HIR.
- **Side-effect ordering inside the RHS list.** Each RHS is
  lowered left-to-right in declaration order — matches how
  `lower_expr` evaluates a single expression. No
  observable side effects in our subset yet (no
  printf-style impure ops in expression position other than
  `print(…)` which is statement-only via `ExprStmt`).
