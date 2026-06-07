# 0180. Function Parameter Table-Context Inference

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-08
- **Deciders:** ShortArrow

## Context

Function parameters in HIR default to `ValueKind::Number` (`src/hir/mod.rs:1248`, `1032`). The body-walk inference (`infer_param_kinds`, line 1030) refines param kind in one direction only: a parameter used in **callee position** is marked `Function(arity)`. Every other usage leaves the param as Number.

This default breaks idiomatic patterns where a parameter is treated as a Table:

```lua
local function takes_table(t)
  for k, v in pairs(t) do return v end   -- for-in-pairs requires lhs=table; rejects t (number)
end
```

The pattern surfaced during ADR 0179's C2 test authoring; we had to rewrite the helper to use an upvalue-captured table instead of a parameter — recorded in the sweep retrospective §Lessons #5.

A second inference pass (`infer_user_function_param_kinds`, line 746) refines params from literal call-site argument kinds. It catches cases like `pick(other)` where `other` is statically known. But it does NOT propagate through the body, and when the body itself constrains the param (e.g. `pairs(t)`), the call-site pass doesn't see that constraint.

The structural fix is symmetric to the existing Function(arity) detection: walk the body for **Table-consumer** uses of a parameter and mark it Table.

## Scope (literal)

- ✅ `ForPairs { table: Ident(param) }` — `for k, v in pairs(param) do ... end`.
- ✅ `ForIpairs { table: Ident(param) }` — `for i, v in ipairs(param) do ... end`.
- ✅ `Index { target: Ident(param), .. }` — `param[k]`, `param.field`.
- ✅ `MethodCall { receiver: Ident(param), .. }` — `param:method(...)`.
- ✅ `Call { callee=Ident(B), args[0]=Ident(param) }` where B is a Table-consumer builtin: `pairs`, `ipairs`, `next`, `setmetatable`, `getmetatable`, `rawget`, `rawset`, `rawequal`, `rawlen`.
- ✅ Conflict policy: last-write-wins. If a body marks the param both `Function(arity)` and `Table`, the textually-later mark survives. Users can re-order or pick a single usage pattern — Lua doesn't have static types and the compiler infers optimistically.
- ❌ Cross-procedure inference (parameter X passed to function Y whose param is also unknown). Each function infers locally; no fixpoint.
- ❌ `String` / `Bool` / other kinds — only Table is added in this ADR. Other kinds can be added incrementally (each is a single arm).
- ❌ Compound expression target detection (e.g. `t[a].field` where `a` is the param). Out of scope — covered by the future expr-position Index widening ADR.

## Decision

### HIR side (`src/hir/mod.rs`)

In `infer_param_kinds`'s `visit_stmt` / `visit_expr` walker, add Table-marking arms:

1. **ForPairs / ForIpairs arm** (extend existing arms):
   ```rust
   StmtKind::ForPairs { table, body, .. } | StmtKind::ForIpairs { table, body, .. } => {
       mark_ident_as_table(table, name_to_idx, kinds);
       visit_expr(table, name_to_idx, kinds);
       for s in body { visit_stmt(s, name_to_idx, kinds); }
   }
   ```

2. **Call to Table-consumer builtin** (extend existing Call arm):
   ```rust
   ExprKind::Call { callee, args } => {
       if let ExprKind::Ident(name) = &callee.kind {
           if let Some(&idx) = name_to_idx.get(name.as_str()) {
               kinds[idx] = ValueKind::Function(args.len());
           } else if is_table_consumer_builtin(name)
               && let Some(first) = args.first() {
               mark_ident_as_table(first, name_to_idx, kinds);
           }
       }
       visit_expr(callee, name_to_idx, kinds);
       for a in args { visit_expr(a, name_to_idx, kinds); }
   }
   ```

3. **Index target arm** (new):
   ```rust
   ExprKind::Index { target, key } => {
       mark_ident_as_table(target, name_to_idx, kinds);
       visit_expr(target, name_to_idx, kinds);
       visit_expr(key, name_to_idx, kinds);
   }
   ```

4. **MethodCall receiver arm** (extend existing):
   ```rust
   ExprKind::MethodCall { receiver, args, .. } => {
       mark_ident_as_table(receiver, name_to_idx, kinds);
       visit_expr(receiver, name_to_idx, kinds);
       for a in args { visit_expr(a, name_to_idx, kinds); }
   }
   ```

5. **Helpers**:
   ```rust
   fn mark_ident_as_table(e: &Expr, map: &Map<&str, usize>, kinds: &mut Vec<ValueKind>) {
       if let ExprKind::Ident(name) = &e.kind {
           if let Some(&idx) = map.get(name.as_str()) {
               kinds[idx] = ValueKind::Table;
           }
       }
   }

   fn is_table_consumer_builtin(name: &str) -> bool {
       matches!(name,
           "pairs" | "ipairs" | "next"
           | "setmetatable" | "getmetatable"
           | "rawget" | "rawset" | "rawequal" | "rawlen")
   }
   ```

The existing call-site refinement pass (`infer_user_function_param_kinds`) runs after the body-walk and **does not overwrite Table → Number**: the merge at line 1469-1474 only assigns when the body-walk left Number AND a literal call-site provided a concrete kind. Body-walked Table sticks.

### Tests

`tests/phase2_6plus_param_table_inference.rs` (NEW, ~4 e2e):
1. `pairs(param)` — function takes a table parameter and iterates with pairs.
2. `ipairs(param)` — same with ipairs.
3. `param[k]` — Index access.
4. `param:method(...)` — MethodCall receiver.

## Alternatives considered

- **Default params to TaggedValue instead of Number.** Rejected — breaks every existing Number-arithmetic param (`function double(x) return x * 2 end` would have to coerce TaggedValue→f64 at every use, a code-shape change across most existing tests). Anti-ad-hoc in principle but disruptive in practice; ADR with much wider scope.
- **Cross-procedure inference fixpoint.** Rejected — disproportionate to the blocker. Local body-walk catches the common patterns.
- **Explicit type annotations.** Rejected — Lua is dynamically typed; the compiler should infer.

## Consequences

**Positive**
- Idiomatic table-parameter patterns work (`pairs(param)`, `param[k]`, `param:method()`).
- The retrospective §Lessons #5 workaround (upvalue capture) becomes unnecessary for the affected pattern.
- Inference is local (no cross-procedure complexity).

**Negative**
- A param used inconsistently in body (callee position then table position) gets last-write-wins kind; may produce confusing TypeMismatch errors. Trade-off: predictable order > rejecting ambiguous patterns.
- Adds 5 arms to the body-walker; small surface increase.

**Locked in until superseded**
- Last-write-wins conflict policy.
- Builtin enumeration: `pairs | ipairs | next | setmetatable | getmetatable | rawget | rawset | rawequal | rawlen`. Adding a builtin requires the enumeration to grow.

## Documentation updates

- [x] §8 — adds 0180.
- [x] Sweep retrospective — "Next chokepoint candidates" parameter inference RESOLVED for Table case (other kinds still future).

## Test count delta

```
Step 0: 1400 (after ADR 0179)
C1 (doc): 1400 → 1400
C2 (4 e2e Red Day 0): 1400 → 1400
C3 (HIR impl): 1400 → 1404
```

## Critical files

- `src/hir/mod.rs`:
  - Extend `infer_param_kinds`'s body-walker with the 4 marking arms.
  - Add `mark_ident_as_table` + `is_table_consumer_builtin` helpers.
- `tests/phase2_6plus_param_table_inference.rs` (NEW) — 4 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Existing tests using a param as both callee and Table break | Last-write-wins is documented; existing 1400 tests pin actual behaviour. Body-walk order is deterministic. |
| Call-site refinement pass overwrites body-walk Table | Confirmed it does not — merge only writes when body-walk left Number AND a kind was observed. |
| Table-consumer enumeration drift (new builtin added later) | Helper function centralises the list; adding a builtin = 1-line edit. |
| Param used as Number arithmetic AND as Table in same body | Last-write-wins — predictable. User can split into two functions. |

## Future work

- String / Bool / Function-arg inference (mirror this ADR for each kind).
- Cross-procedure inference (fixpoint over call graph).
- Expr-position `Index` widening (separate, larger ADR — touches ADR 0054 chokepoint).

## References

- [Sweep retrospective 0166-0177](../notes/sweep-0166-0177-retrospective.md) — §Next chokepoint candidates surfaced this gap.
- [ADR 0094](0094-phase2-method-idx-call-refine.md) — `infer_user_function_param_kinds` precedent.
- [ADR 0179](0179-non-local-tagged-source-materialisation.md) — discovered the blocker during test authoring.
- Lua 5.4 reference §3.4.5 / §6.1 — pairs/ipairs/next semantics.
