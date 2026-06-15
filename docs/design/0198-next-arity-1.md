# 0198. `next(t)` arity 1 — Implicit `nil` Second Argument

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Lua 5.4 §6.1 (basic library): `next(table [, index])`. The second arg is **optional**; if omitted, it defaults to `nil` (signal to return the first key). The current LuMeLIR locks `Builtin::Next` to arity `(2, 2)` (`src/hir/ir.rs:743`), forcing every call site to write `next(t, nil)` explicitly.

[Bucket E probe results](../notes/bucket-e-probe-results.md) §E8 surfaced this gap: `print(next(t))` fails with `ArityMismatch { builtin: "next", expected: 2, actual: 1 }`. The leftover-roadmap memo identified this as the smallest of bucket E's three remaining gaps.

## Scope (literal)

- ✅ Relax `Builtin::Next` arity from `(2, 2)` to `(1, 2)`.
- ✅ When `args.len() == 1`, synthesize a `Nil` HIR literal as `args[1]` inside `lower_namespace_builtin_call` / `lower_builtin_call`'s `Builtin::Next` arm. Downstream code sees the same shape as the explicit `next(t, nil)` case.
- ✅ Test the multi-assign form `local k, v = next(t)` which is the codegen-supported call shape per ADR 0081.
- ❌ Single-value-position `print(next(t))`. Codegen at `emit.rs:9787` rejects single-value `next` for any arity (existing ADR 0081 scope); orthogonal gap, separate ADR if needed.
- ❌ Iteration via `for k, v in next, t do ... end` (Lua's "stateless iterator" form). Uses generic-for protocol; if it works today, regression-pin; if not, separate ADR.
- ❌ Numbering shift for ADR 0196 §Sub-ADR decomposition: this ADR consumes slot 0198; Integer/Float plan now reads 0199-0207 (shift by 1).

## Decision

### `src/hir/ir.rs`

```rust
Builtin::Next => (1, 2),
```

(was `(2, 2)`)

### `src/hir/mod.rs::lower_builtin_call`

After `lowered_args` is built and before the `next`-specific arg validation (line ~5154), append `HirExprKind::Nil` when `args.len() == 1 && matches!(builtin, Builtin::Next)`:

```rust
if matches!(builtin, Builtin::Next) && lowered_args.len() == 1 {
    let span = whole.span;
    lowered_args.push(HirExpr {
        kind: HirExprKind::Nil,
        span,
    });
}
```

The downstream argument-kind validation sees Nil at position 1, which is a valid "no previous key" sentinel for `next` per Lua spec. No further change needed.

### Test

`tests/phase4_next_arity_1.rs` (NEW, 1 e2e):

```lua
local t = {}
rawset(t, "a", 1)
local k, v = next(t)
print(k, v)
```

Expected: `"a\t1"` (the only key-value pair in the table).

## Alternatives considered

- **Synthesize Nil at the parser instead of HIR.** Rejected — the parser is shared with every other builtin; surfacing the synthesis there leaks `Next`-specific knowledge. HIR `lower_builtin_call` is the right layer.
- **Codegen-level default.** Rejected — by codegen time the arg count is already locked in by HIR arity validation. Earlier is cleaner.
- **Both arity forms via overloading.** Rejected — Lua spec defines a single arity-range function; overloading is the wrong shape.

## Consequences

**Positive**
- Idiomatic Lua `next(t)` works without forcing users to write `next(t, nil)`.
- Bucket E count drops from 3 → 2.

**Negative**
- One more arity-flexible builtin. Trivial maintenance cost.

**Locked in until superseded**
- Builtin::Next arity is now (1, 2). Future ADRs adding more relaxed forms (e.g. `next(t, k1, k2)`) update this.

## Documentation updates

- [x] §8 — adds 0198.
- [x] `docs/notes/bucket-e-probe-results.md` — marks E8 RESOLVED in a follow-up doc edit.
- [x] ADR 0196 §Sub-ADR decomposition — numbering shifts; future ADRs use 0199-0207 for Integer/Float arc.

## Test count delta

```
Step 0: 1437 (after b7343dc)
C1 (doc): 1437 → 1437
C2 (1 Red Day 0 e2e): 1437 → 1437 (Red — ArityMismatch as probed)
C3 (impl): 1437 → 1438 (Green)
```

## Critical files

- `docs/design/0198-next-arity-1.md` (this doc).
- `docs/design/README.md` index entry.
- `src/hir/ir.rs::arity` — change Builtin::Next arity tuple.
- `src/hir/mod.rs::lower_builtin_call` — synth Nil for arity-1 case.
- `tests/phase4_next_arity_1.rs` (NEW) — 1 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Synthesized Nil arg breaks downstream codegen | `next(t, nil)` is the existing call shape; codegen already handles it via the ForPairs desugar (ADR 0081). |
| Single-value `next(t)` use surfaces ADR 0081 codegen rejection | Documented out of scope; user gets the existing rejection error, not ArityMismatch. |
| ADR 0196 plan numbering drift | Single shift by 1; cross-reference here is the only update needed (no automated audit fails). |

## Future work

- ADR (TBD) — Single-value-position `next(t)` (`print(next(t))`). Drops the ADR 0081 scope ceiling. Larger surface (codegen tagged-key truncation).
- ADR (TBD) — Stateless iterator form `for k, v in next, t do ... end`. Probe first.

## References

- [Lua 5.4 Reference Manual §6.1 next](https://www.lua.org/manual/5.4/manual.html#pdf-next)
- [ADR 0081](0081-phase2-8e-iter-next.md) — `Builtin::Next` introduction with arity (2, 2).
- [Bucket E probe results](../notes/bucket-e-probe-results.md) — E8.
- [Leftover roadmap](../notes/leftover-roadmap.md) — bucket E.
