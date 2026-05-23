# 0050. Phase 2.1b: Multi-Target Reassignment from a Multi-Result Call

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

ADR 0049 (Phase 2.1a) added parallel-evaluation
multi-target reassignment: `a, b = b, a`. ADR 0049's
"Out of Scope" carved out the multi-result Call form:

```lua
local function pair() return 10, 20 end
local a = 0
local b = 0
a, b = pair()   -- one Call expanding to N targets
```

Phase 2.5d (ADR 0021) already had the symmetric form for
`local`:

```lua
local a, b = pair()   -- works — uses MultiAssignFromCall HIR node
```

So the gap was purely in the non-`local` reassignment path.
The HIR node + codegen path already exists; only HIR
lowering needed extension.

## Decision

### Detect 1-RHS-Call shape in `lower_assign_multi`

Mirror `lower_local_multi`'s shape, but resolve / auto-
declare targets instead of declaring fresh:

```rust
if values.len() == 1 && names.len() > 1 {
    let lowered = self.lower_expr(&values[0])?;
    if let HirExprKind::Call { callee, args } = lowered.kind {
        let ret_kinds: Vec<ValueKind> = match callee {
            Callee::User(FuncId(fid)) => self.functions[fid].ret_kinds.clone(),
            _ => return Err(HirError::ArityMismatch { … }),
        };
        if ret_kinds.len() != names.len() { return Err(…); }
        let dst_ids = self.resolve_or_declare_multi_targets(names, &ret_kinds, span)?;
        return Ok(HirStmt {
            kind: HirStmtKind::MultiAssignFromCall { dst_ids, callee, args },
            span,
        });
    }
    // Single non-Call RHS with N>1 targets: fall through to
    // the arity-mismatch error path of the parallel branch.
}
```

The shortcut requires `Callee::User`; builtins
(`Callee::Builtin`) and indirect calls
(`Callee::Indirect`) lack statically-tracked ret kinds, so
they reject as `ArityMismatch` — same pattern Phase 2.5d
already uses for the analogous `local` form.

### Shared resolver: `resolve_or_declare_multi_targets`

Extract the per-target resolve / kind-check / auto-declare
logic into a helper used by the new Call path. The original
parallel path (Phase 2.1a) duplicates this work inline; rule
of three not yet met (just two sites). If a third write
site emerges, the parallel path can adopt the helper.

### Auto-declare per ADR 0048

When a target name doesn't resolve and we're at chunk top
level, the helper auto-declares it at chunk scope.
Function-body unresolved names error with `UndefinedName`,
matching the single-target rule.

### CA invariants preserved

| Layer    | Change                                            |
|----------|---------------------------------------------------|
| Lexer    | None                                              |
| Parser   | None — AST shape from 2.1a unchanged              |
| AST      | None                                              |
| HIR      | `lower_assign_multi` extended; new helper `resolve_or_declare_multi_targets` |
| Codegen  | None — uses existing `MultiAssignFromCall` emit  |

The change is HIR-only. Codegen's `emit_multi_assign_from_call`
(written for 2.5d's `local` form) stores into
`slots[dst.0]`; whether the slot was just declared by 2.5d
or re-used by 2.1b is invisible to codegen.

## TDD Process

1. **Red.** 7 e2e tests covering Call-to-existing-locals,
   Call-to-globals (auto-declare), Call-arity-mismatch,
   builtin-call rejection, target-kind mismatch, chained
   use, and a 2.1a parallel-form regression. 3 failed (the
   new behaviour); 4 already passed (boundary rejections via
   the 2.1a arity-mismatch path).
2. **Green.** `lower_assign_multi`'s Call shortcut + the
   resolver helper. All 7 tests pass at 649 (642 + 7).
3. **Refactor.** None warranted — the parallel path's
   inline target-resolve logic stays for now (rule of three
   not met).

## Alternatives Considered

- **Reuse the parallel-path's temp-then-assign machinery**
  by lowering the Call into N `_multi_tmp_N` temps, then
  storing each temp into its target. Works, but emits an
  extra alloca + load + store per target compared to
  reusing `MultiAssignFromCall`'s direct result-to-slot
  store. Rejected for the wasted IR.
- **Allow `a, b, c = pair(), 5`** — Lua's actual rule lets
  the last value expression be a Call that fills the
  remaining slots. Adds non-trivial parser/HIR complexity
  and isn't a common pattern. Defer.
- **Allow `Callee::Indirect`** (function passed as a value)
  by tracking ret arity per function-typed local. Doable but
  needs `LocalInfo` to carry ret_kinds, not just arity. The
  same gap blocks the `local` form. Defer to a phase that
  unifies first-class function-value tracking.

## Consequences

- HIR adds ~70 lines (the Call-shortcut branch + helper).
- 7 new e2e tests; total green at 649.
- Idiomatic Lua patterns like `mid, score = compute()`
  now work without `local`.

## Out of Scope

- **Mixed expression forms** (`a, b, c = pair(), 5`).
- **Indirect-callee multi-result** — pending ret-arity
  tracking on Function-kind locals.
- **Builtin multi-result** (none of our builtins return
  more than one value yet).
- **Reorganising `lower_assign_multi` and the parallel path
  to share the resolver** — pending a third write site.
