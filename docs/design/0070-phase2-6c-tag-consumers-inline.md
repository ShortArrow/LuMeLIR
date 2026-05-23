# 0070. Phase 2.6c-tag-consumers-inline: Inline `type(t[k])` / `tostring(t[k])` Runtime Dispatch

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

ADR 0067 routed `type(Local(TaggedValue))` and
`tostring(Local(TaggedValue))` through runtime tag dispatch and
explicitly listed the inline forms (`type(t[k])`,
`tostring(t[k])`) as out of scope:

> Inline `type(t[k])` and `tostring(t[k])` — current scope is
> `Local(TaggedValue)` only. Inline forms still go through the
> static `Index` path.

That residual surface was tracked as **LIC-2.6c-tag-consumers-
inline-1**. Codex review (post-ADR-0069) flagged it as the
natural follow-up after the ADR 0069 trap consolidation: the
inline path is the missing twin of the local path and silently
mis-types Bool / String values, traps on Nil from OOB / missing
key, and breaks `..` concat with inline indexed strings.

The fix mirrors the `Builtin::Print` inline pattern from ADR
0065: when the operand is a `HirExprKind::Index`, materialise
the read into a tmp 16-byte tagged slot and dispatch through
the existing tag-aware helper. No new helpers required — both
`emit_local_init_tagged` (ADR 0063) and the consumer-side
`emit_type_tagged_local` / `emit_tostring_tagged_local` (ADR
0067) already exist.

## Decision

### `Builtin::Type` arm — Index special case

In `Callee::Builtin(Builtin::Type)`, after the existing
`Local(TaggedValue)` special case and **before** the static-
dispatch fallback, intercept the `HirExprKind::Index` shape:

```rust
if let HirExprKind::Index { target, key } = &args[0].kind {
    let tmp = emit_alloca_slot_for_kind(
        context, block, ValueKind::TaggedValue, types, loc,
    );
    emit_local_init_tagged(
        context, block, tmp, target, key, slots, locals,
        functions, types, params_len, loc,
    )?;
    return Ok(emit_type_tagged_local(context, block, tmp, types, loc));
}
```

### `Builtin::ToString` arm — Index special case

Identical pattern, swapping `emit_type_tagged_local` for
`emit_tostring_tagged_local`:

```rust
if let HirExprKind::Index { target, key } = &args[0].kind {
    let tmp = emit_alloca_slot_for_kind(
        context, block, ValueKind::TaggedValue, types, loc,
    );
    emit_local_init_tagged(
        context, block, tmp, target, key, slots, locals,
        functions, types, params_len, loc,
    )?;
    return Ok(emit_tostring_tagged_local(context, block, tmp, types, loc));
}
```

### Concat (`..`) auto-coerce inheritance

ADR 0026 wraps non-String operands of `..` in a `tostring(...)`
HIR call. With the new inline `tostring(Index)` dispatch, every
`"prefix" .. t[k]` (or symmetric) immediately gains the runtime
tag dispatch — no code changes outside `Builtin::ToString`.
Matrix tests cover `concat_inline_string` and
`concat_inline_bool`.

### Out-of-scope consumers

Other builtins that touch `Index` operands (`assert`, `error`,
`tonumber`) keep their current static-dispatch / `emit_expr`
paths. Of these:

- `assert(t[k])` — currently HIR-rejects unless the operand has
  Bool kind; out of scope.
- `error(t[k])` — String operand expected; an inline Index in
  that position is unusual.
- `tonumber(t[k])` — already handles the Number / String static
  cases; inline Index still extracts as f64 and traps on
  non-Number. Worth a follow-up sub-phase if profiling demands.

The same logic applies to arithmetic / comparison operators on
inline `Index`: those trap on non-Number and that is Lua-spec
correct. No expansion.

### CA invariants preserved

| Layer    | Change                                                        |
|----------|---------------------------------------------------------------|
| Lexer    | None                                                          |
| Parser   | None                                                          |
| AST      | None                                                          |
| HIR      | None                                                          |
| Codegen  | Two arm extensions in `emit.rs` (`Builtin::Type` /            |
|          | `Builtin::ToString`); no new helper functions.                |

## TDD Process

1. **Step 1 — Red.** 14 e2e tests in
   `tests/phase2_6c_tag_consumers_inline_matrix.rs` covering:
   - `type(inline Index)` × {Number, Bool, String, Nil-OOB,
     hash String, hash missing} — 6 cells
   - `tostring(inline Index)` × {Number, Bool, String, Nil-OOB}
     — 4 cells
   - `..` concat with inline Index × {String, Bool} — 2 cells
   - regression for ADR 0067 `Local(TaggedValue)` path — 2 cells
   10 fail outright (the new behaviour), 4 pass (regression
   coverage).
2. **Step 2 — Green.** Two arm extensions land. All 14 pass;
   total green at **836** (= 822 + 14).
3. **Step 3 — ADR + AGENTS + commit.**

## Alternatives Considered

- **Make `infer_kind(Index)` runtime-dispatched (return
  `TaggedValue`).** Forces every consumer of `Index` to handle
  `TaggedValue`, including arithmetic, comparison, length,
  function calls, etc. Massive blast radius; defeats the
  static-kind type system the rest of the compiler relies on.
  Rejected.
- **Extend the special case to every `Builtin`** (`assert`,
  `error`, `tonumber`, …). Out of scope for this sub-phase;
  see "Out-of-scope consumers" above. The two consumers
  resolved here are the ones that LIC-2.6c-tag-consumers-
  inline-1 explicitly named.
- **Pre-Tidy: extract the tmp-tagged-slot pattern into a helper
  before adding the new arms.** `Builtin::Print` already uses
  it (ADR 0065) and now `Builtin::Type` / `Builtin::ToString`
  do too — three call sites. A common helper
  (`emit_inline_index_into_tagged_tmp`?) is the rule-of-three
  trigger. Captured as a follow-up Tidy First; not in this
  ADR's scope to avoid mixing refactor and feature commits.

## Consequences

- ~50 LOC across two arm extensions in `src/codegen/emit.rs`.
  No new helpers.
- 14 new e2e tests. Total green at **836** (= 822 + 14). Zero
  regressions.
- **LIC-2.6c-tag-consumers-inline-1 → resolved.** Inline `type`
  / `tostring` now match runtime tag, mirroring ADR 0067 for
  the local path.
- `tagged-semantics.md` §3 (consumer matrix `type` / `tostring`
  rows lose the `*` LIC marker on inline Index), §4 (LIC moves
  from Pending to Resolved, total now 9 resolved / 3 partial /
  1 pending), §7 (open question removed), §8 (ADR 0070 added).
- Follow-up Tidy First candidate: extract the
  `Index → tmp tagged slot → consumer` pattern into a helper
  (rule-of-three: Print, Type, ToString).

## Documentation updates

- [x] §1 slot layout — n/a
- [x] §2 producer / source taxonomy — n/a (no new producer)
- [x] §3 consumer coverage matrix — `type` / `tostring` inline
      rows updated to runtime dispatch; `*` LIC marker removed
- [x] §4 LIC consolidation — `LIC-2.6c-tag-consumers-inline-1`
      moved from Pending to Resolved; totals updated
- [x] §5 runtime tag invariants — n/a
- [x] §7 open questions — inline `type` / `tostring` item removed
- [x] §8 ADR index — ADR 0070 added; "Last updated" bumped

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068 / Codex review).
