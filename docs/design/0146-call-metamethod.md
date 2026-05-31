# 0146. `__call` Metamethod (Callable Table)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Tier 2 metamethod ADR #4. Lua spec §3.4.10: applying `t(args)` where `t` is not a Function consults `t.__call` and invokes it as `t.__call(t, args)` — `t` becomes the first argument (the `self`).

Today `lower_call` (`src/hir/mod.rs`) rejects `t()` when `t` resolves to a Table-kind Local with `UnknownFunction` — there is no `Callee` arm for "Table local as callee".

With [ADR 0141](0141-anon-fn-indexassign-param-refine.md) (anon-fn param refinement) and [ADR 0142](0142-tostring-metamethod.md)'s `emit_dispatch_chain_from_slot_ptr` helper landed, this is the smallest remaining metamethod that wires through the existing call ABI.

## Scope (literal)

**`t(args)` rewritten to `t.__call(t, args)` at HIR**, where `t` is a Table-kind Local. The rewrite goes through ADR 0091 → ADR 0082 dispatch — no new codegen path.

Concrete restrictions:

- ❌ Multi-segment Table receiver (`obj.inner(args)` rewriting `inner.__call`). Single-Ident Table receiver only.
- ❌ Variadic `__call`. Each call site arity must match a candidate signature exactly.
- ❌ Non-Function `__call` metafield (Lua spec allows recursing — e.g. `__call` can itself be a Table). Function-only here.
- ❌ TaggedValue runtime Table-tag dispatch. Static-Table only.
- ❌ Multi-return `__call`.

## Decision

### HIR rewrite

`lower_call` adds an early arm BEFORE the existing namespace-builtin / ADR 0091 callee-classify path:

```rust
if let ExprKind::Ident(name) = &callee.kind
    && let Some(local_id) = self.resolve(name)
    && matches!(self.locals[local_id.0].kind, ValueKind::Table)
{
    // Rewrite: t(args) → t.__call(t, args...)
    let recv_expr = Expr::new(ExprKind::Ident(name.clone()), callee.span);
    let key_expr = Expr::new(ExprKind::Str("__call".to_owned()), callee.span);
    let index_callee = Expr::new(
        ExprKind::Index {
            target: Box::new(recv_expr.clone()),
            key: Box::new(key_expr),
        },
        callee.span,
    );
    let mut new_args = Vec::with_capacity(args.len() + 1);
    new_args.push(recv_expr);
    new_args.extend_from_slice(args);
    return self.lower_call(&index_callee, &new_args, whole);
}
```

The recursion routes through the existing ADR 0091 IndexCallee branch, which materialises `t.__call` to a synth local and dispatches via ADR 0082 with `(t, args)`.

### Metamethod-aware kind refinement

Extend the post-Pass-1.5 walk for `__call`:

| Key | Forced signature |
|---|---|
| `__call` | `params[0] = Table` (other params left at Pass-1.5 inference) |

Unlike `__tostring` / `__concat` / `__eq`, `__call` arity varies. The walk forces only `params[0]` (the `self` arg). The remaining params inherit whatever Pass-1.5 inferred from the eventual call site — but the call site is `t(args)` (rewritten to `t.__call(t, args)`), which DOES match `Call(Index(Local, Str), [args])` and is visible to the ADR 0094 walker. So the remaining params refine naturally.

### Codegen

No codegen changes. The HIR rewrite reuses the existing ADR 0091 + 0082 dispatch chain.

## Alternatives considered

- **New `Callee::TableCall` variant.** Rejected — the rewrite-to-Index form is HIR-only, zero codegen surface change.
- **Multi-segment receiver (`obj.inner(args)`).** Deferred — needs distinguishing "intermediate field access" from "callable Table".
- **Variadic `__call`.** Deferred — the candidate filter would need per-call-site sig synthesis with arbitrary arity.
- **Implicit `self` injection at codegen instead of HIR.** Rejected — HIR is the natural layer; codegen would need a per-call-site flag, breaking the existing IndirectDispatch ABI.

## Consequences

**Positive**
- `t(args)` works when `t` has a `__call` metamethod — the canonical Lua callable-object idiom.
- Zero codegen surface change. Risk localised to the HIR rewrite arm.
- Refinement reuses the natural Pass-1.5 path because the rewrite produces a `Call(Index(...), ...)` shape that the walker already handles.

**Negative**
- HIR `lower_call` grows one early arm. Care needed: the arm fires ONLY when the resolved Local has kind Table, so existing Function-kind callees (the dominant case) hit zero overhead.
- Multi-segment receiver path (`obj.inner()` where `inner` is a callable Table) remains rejected. Deferred to a follow-up.

**Locked in until superseded**
- Single-Ident receiver only.
- Function-form only.
- Static arity at each call site (no variadic candidates).

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC — new `LIC-call-metamethod-1`.
- [x] §7 open questions — closes `__call` Table-receiver item; opens multi-segment / variadic / TaggedValue-runtime-Table as new follow-ups.
- [x] §8 ADR index — adds 0146.

## Test count delta

```
Step 0:   1330 (after ADR 0145)
C2 (4 e2e Red Day 0):  1330 → 1330
C3 (impl): 1330 → 1334
```

## Critical files

- `src/hir/mod.rs`:
  - `lower_call` adds the Table-Ident-callee rewrite arm.
  - Metamethod-aware refinement walk extends `__call` arm (forces `params[0] = Table`).
- `tests/phase2_6plus_call_metamethod.rs` (NEW) — 4 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Existing Function-kind callee gets accidentally rewritten | The arm matches only when `self.locals[id].kind == ValueKind::Table`. Function locals hit zero overhead. |
| `t.method(args)` (Index-callee, not Ident) accidentally hits the rewrite | The arm checks `ExprKind::Ident` strictly; non-Ident callees fall through to the existing path. |
| Multi-arg `__call` candidate mismatch | The Pass-1.5 walker refines `params[1..]` from the rewritten `Call(Index(t, "__call"), [t, args])` site. As long as candidates match, dispatch succeeds. Tests pin. |
| Stack overflow if `__call` is recursive | Same as any user-fn recursion. Deferred. |

## Future work

- Multi-segment Table receiver (`obj.inner(args)` where `inner.__call` is a metamethod).
- Variadic `__call`.
- Non-Function `__call` (Lua spec allows `__call` to recursively chain).
- TaggedValue runtime Table-tag dispatch.
- Multi-return `__call`.

## References

- [ADR 0082](0082-phase2-5x-callee-dispatch.md) — IndirectDispatch chain.
- [ADR 0091](0091-phase2-callee-normalization.md) — Index-callee Call synth-local.
- [ADR 0141](0141-anon-fn-indexassign-param-refine.md) — anon-fn param refinement.
- [ADR 0142](0142-tostring-metamethod.md) — `emit_dispatch_chain_from_slot_ptr` helper (indirectly used via ADR 0082).
- Lua 5.4 reference manual §3.4.10 — `__call` semantics.
