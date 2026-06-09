# 0183. MethodCall on String Receiver — Namespace Builtin Dispatch

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-09
- **Deciders:** ShortArrow

## Context

[ADR 0103](0103-stdlib-string-begin.md) added the `string.<method>(s, ...)` call form via the `Builtin::from_namespace_method` chokepoint. Real-world Lua code more often uses the method-syntax sugar `s:<method>(...)`, which Lua semantics expand to `string.<method>(s, ...)` via the string metatable (`__index = string`).

`tests/phase2_stdlib_string.rs:19` already documents `s:len()` method syntax as deferred ("needs metatable"). Today the existing MethodCall lowering at `src/hir/mod.rs:4451` synthesises `Call { Index { recv, Str(method) }, [recv, ...args] }` and recurses through `lower_call`'s Index-callee path. For a String-kind receiver this fails downstream: `s["upper"]` is not a meaningful Index expression (Strings are not tables in LuMeLIR), so the eventual IndirectDispatch has no candidate.

[ADR 0181](0181-param-string-context-inference.md) introduced parameter String-context inference at Concat and `string.<method>(param)` sites, but left the MethodCall form as deferred future work. [ADR 0182](0182-param-inference-kind-parameterised-helpers.md) consolidated the body-walker into a kind-parameterised helper, paving the way for the inference fold-through here at one-line cost.

## Scope (literal)

- ✅ **String receivers only**. `local function up(s) return s:upper() end` and the seven other recognised `string.<method>` names (`len` / `upper` / `lower` / `sub` / `rep` / `byte` / `char` / `format`).
- ✅ **MethodCall lowering arm**: when the receiver resolves to a Local of `ValueKind::String` and `Builtin::string_from_method(method)` is `Some(_)`, route through `lower_namespace_builtin_call` with `[recv, ...args]`. Otherwise fall through to the existing Index-callee path (zero regression).
- ✅ **MethodCall inference arm**: when method ∈ string-method set → mark the receiver param as `String`. Otherwise keep the existing default `Table` mark (ADR 0180).
- ❌ **Table receivers** (`t:concat(",")`, `t:insert(x)`, `t:remove()`). Table-side short-circuit is structurally unsafe — users can define `function t:concat(x) return x end`, in which case the user method must win. Static HIR has no way to know the table's runtime method-set without inspecting `IndexAssign` history. Deferred to a future ADR once shadowing detection is in place.
- ❌ **Bool / Number / Nil receivers**. No primitive method namespace.
- ❌ **MethodCall on non-Ident receivers** (Call / FunctionExpr / etc.) — already rejected at the existing `check_method_receiver_shape` choke; this ADR does not relax that.
- ❌ **`__index = string`-style metatable** dispatch for user-defined wrappers. Distinct mechanism, future ADR.
- ❌ **Cross-procedure receiver-kind inference** — relies on the ADR 0180/0181 body-walk only.

## Decision

### `src/hir/mod.rs`

#### 1. MethodCall lowering arm extension (line ~4451)

Before the existing `materialize_to_synth_local` / Index-callee synth path, query the receiver's resolved kind. If the receiver is a Local with `ValueKind::String` and the method is recognised by `Builtin::string_from_method`, dispatch directly:

```rust
let recv_id = self.resolve(&recv_name);
if let Some(id) = recv_id
    && matches!(self.locals[id.0].kind, ValueKind::String)
    && let Some(builtin) = Builtin::string_from_method(&method)
{
    let mut ns_args = Vec::with_capacity(args.len() + 1);
    ns_args.push(Expr::new(ExprKind::Ident(recv_name.clone()), receiver.span));
    for a in args { ns_args.push(a.clone()); }
    return Ok(self.lower_namespace_builtin_call(builtin, &ns_args, expr.span)?);
}
```

Receivers that miss any of the three conditions (no resolve / non-String kind / unknown method) fall through to the existing Call-Index synth, preserving every current path including ADR 0091's Index-callee dispatch and ADR 0092's MethodCall regression net.

#### 2. MethodCall inference arm refinement (line ~1261)

ADR 0182's `mark_ident_as` takes the kind as an argument. Refine MethodCall's mark to be method-name-aware:

```rust
ExprKind::MethodCall { receiver, method, args } => {
    let recv_kind = if Builtin::string_from_method(method).is_some() {
        ValueKind::String
    } else {
        ValueKind::Table
    };
    mark_ident_as(receiver, name_to_idx, kinds, recv_kind);
    visit_expr(receiver, name_to_idx, kinds);
    for a in args { visit_expr(a, name_to_idx, kinds); }
}
```

The current "always Table" was a deliberate ADR 0180 default; widening to method-aware refinement is safe because `is_body_decisive_kind` (ADR 0182) already permits both String and Table to override the call-site default.

### Tests

`tests/phase2_6plus_param_methodcall_string.rs` (NEW, 3 e2e):
1. `param:upper()` → returns uppercase
2. `param:lower()` → returns lowercase
3. `param:len()` → returns length (Number)

Existing 1407 corpus is the regression net — in particular `tests/phase2_method_syntax.rs` (ADR 0092 MethodCall on Table) and `tests/phase2_stdlib_string.rs` (the `string.<method>(s)` form).

## Alternatives considered

- **Receiver-kind-indexed dispatcher `Builtin::from_receiver_kind_method(kind, method)`**. Adding this preemptively with only one populated arm (String) is speculative. Will become natural Tidy First once the Table side lands (ADR 0184+). Tracking as future work.
- **Always short-circuit on method name without checking receiver kind.** Rejected — would dispatch `t:upper()` (user-defined method on a Table) to `Builtin::StringUpper`, producing a runtime crash. Receiver-kind gate is the safe minimum.
- **Add `__index = string` metatable on String values at runtime.** Rejected for scope — far larger than this ADR's narrow MethodCall sugar; would require a runtime metatable slot on Strings (currently absent).
- **Extend ADR 0181's body-walker to also infer String from `t:upper()` even when method is on a TaggedValue.** Rejected — confuses inference with dispatch; the lowering path needs the same gate regardless.

## Consequences

**Positive**
- `s:upper()` / `s:lower()` / `s:len()` / `s:sub(i, j)` / `s:rep(n)` / `s:byte(i)` / `s:format(...)` all work for parameters and locally-bound Strings.
- Mirrors ADR 0103's namespace dispatch chokepoint — same `from_namespace_method` mechanism reached from the dual entry point.
- Removes the `tests/phase2_stdlib_string.rs:19` "deferred" annotation in spirit (the docs note can be relaxed in a follow-up).
- Inference fold-through completes the ADR 0181 pattern with one extra arm.

**Negative**
- MethodCall lowering grows a 4-line receiver-kind probe before the synth path. Acceptable — symmetric to the namespace `extract_namespace_call` gate in `lower_call`.
- Table side intentionally asymmetric until shadowing detection lands. Documented.

**Locked in until superseded**
- Receiver-kind check uses `self.locals[id.0].kind` — same provenance source ADR 0180/0181 rely on. Future ADRs adding cross-procedure kind refinement would update it uniformly.

## Documentation updates

- [x] §8 — adds 0183.
- [x] ADR 0103 cross-reference — dual entry point to namespace dispatch.
- [x] ADR 0181 future-work — MethodCall String fold-through marked DONE.
- [x] ADR 0182 future-work — first kind-extending consumer; receiver-kind-indexed dispatcher candidate for ADR 0184+ when Table arrives.

## Test count delta

```
Step 0: 1407 (after ADR 0182)
C1 (doc): 1407 → 1407
C2 (3 e2e Red Day 0): 1407 → 1407
C3 (HIR impl): 1407 → 1410
```

## Critical files

- `src/hir/mod.rs`:
  - MethodCall lowering arm — add receiver-kind + method-name gate before synth path.
  - MethodCall inference arm — refine the mark via `string_from_method` test.
- `tests/phase2_6plus_param_methodcall_string.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Receiver wrongly typed as String triggers spurious StringUpper dispatch | Receiver-kind gate uses the body-walker's own inference; if it says String, dispatch is consistent with the inferred kind. Misinference would already break the param at other use sites. |
| ADR 0091 (Index-callee) or ADR 0092 (MethodCall via Table) regress | Gate is additive — when any of the three conditions miss, the existing synth path runs unchanged. Regression nets: phase2_index_callee, phase2_method_syntax. |
| User shadows `string` namespace and expects `s:upper()` to call their version | Out of scope. `local string = {}` shadowing applies to the `string.<method>(s)` form via `self.resolve("string")`. For the method-syntax form there is no `string` identifier in the path; the dispatch is keyed on the receiver's kind. Documented. |
| Future Table-side ADR breaks the symmetric structure | When the Table side lands, both arms will be folded into `from_receiver_kind_method` as a Tidy First refactor (ADR 0182 precedent). |

## Future work

- **ADR 0184 = MethodCall on Table receiver, namespace builtins (`t:concat(",")`)** — needs `IndexAssign` shadowing detection to avoid clobbering user-defined methods.
- **Receiver-kind-indexed dispatcher** (`Builtin::from_receiver_kind_method`) — Tidy First refactor once two arms (String + Table) exist.
- **Metatable-driven MethodCall** — full Lua semantics for `__index = string` user-defined wrappers.
- **Lift the `tests/phase2_stdlib_string.rs:19` deferred annotation** in a documentation follow-up.

## References

- [ADR 0103](0103-stdlib-string-begin.md) — `string.<method>(s)` namespace dispatch (dual entry point).
- [ADR 0091](0091-phase2-6plus-callee-norm.md) — Index-callee normalisation; this ADR adds a pre-step.
- [ADR 0092](0092-phase2-6plus-method-syntax.md) — MethodCall synth-receiver path (preserved as fall-through).
- [ADR 0180](0180-param-table-context-inference.md) — body-walker introduction.
- [ADR 0181](0181-param-string-context-inference.md) — String-context inference; this ADR closes its MethodCall future-work.
- [ADR 0182](0182-param-inference-kind-parameterised-helpers.md) — kind-parameterised marker; this ADR is its first consumer.
- Lua 5.4 §3.4.11 — method-call syntactic sugar.
