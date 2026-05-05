# 0065. Phase 2.6c-tag-hetero-fix: Inline Tagged Print + Eq Runtime Dispatch

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

`codex review --commit HEAD` (run on the ADR 0064 commit) flagged
two **P1** issues left over from heterogeneous table values:

1. **`print(t[k])` traps on Bool / String / Nil payloads.** ADR
   0064 routed `print(Local(TaggedValue))` through a runtime tag
   dispatch, but the inline form `print(t[k])` / `print(t.k)` /
   `print(({"x"})[1])` still went through `HirExprKind::Index`'s
   trap-on-non-Number read path. Storing a hetero value via the
   new write API and then reading it inline would abort.
2. **`TaggedValue == "literal"` constant-folds to `false`.** The
   ADR 0061 heterogeneous-kind fold collapses
   `TaggedValue == String` (or `Bool`, `Number`) to a static
   `false` because the static kinds differ. But TaggedValue's
   runtime tag may match the literal — silent miscompile across
   `local x = t[1]; if x == "a" then ...`.

Both issues are wrong-code on otherwise idiomatic Lua. This ADR
fixes both with the smallest viable surface change.

## Decision

### Issue 1 — `print(Index)` materialises through a tmp tagged slot

`Builtin::Print`'s argument loop now special-cases
`HirExprKind::Index { target, key }` ahead of the existing
`emit_expr` path: it allocates a 16-byte tmp slot via
`emit_alloca_slot_for_kind(TaggedValue)`, populates it with the
non-trapping `emit_local_init_tagged` (ADR 0063), then prints it
through `emit_print_tagged_local` (ADR 0064):

```rust
if let HirExprKind::Index { target, key } = &a.kind {
    let tmp_slot = emit_alloca_slot_for_kind(TaggedValue, …);
    emit_local_init_tagged(tmp_slot, target, key, …)?;
    emit_print_tagged_local(tmp_slot, …);
    continue;
}
```

The two helpers are pure reuse — no new MLIR shapes, just a new
caller. Other consumers (arith, comparison, `tostring`, etc.)
keep their trapping `Index` codegen for now; the `print` site is
the one that surfaces the bug to users most directly and is
easy to fix surgically.

### Issue 2 — TaggedValue Eq fold guard + runtime dispatch

Two coordinated changes:

**HIR.** `lower_expr::BinOp::Eq | Ne` now skips the
heterogeneous-kind fold whenever either operand has kind
`TaggedValue`:

```rust
let either_tagged = lk == TaggedValue || rk == TaggedValue;
let fold = !either_tagged && (lk != rk || (lk == Nil && rk == Nil));
```

The `IsNilQuery` / `IsNilLocal` pattern detection that runs
*before* the fold is unchanged — `TaggedValue == Nil` still
lowers to `IsNilLocal` and never reaches the fold path.

**Codegen.** A new helper `emit_tagged_eq_runtime_dispatch`
runs at the start of the `BinOp::Eq | Ne` arm. It detects the
`(Local(TaggedValue), typed)` shape (in either order), reads
the slot's tag, and emits a single-level `scf.if` that:

- in the *then* branch, loads the payload as the typed kind
  (`f64` for Number, `i64`-trunc-to-`i1` for Bool, `ptr` then
  `strcmp` for String) and compares against the typed RHS;
- in the *else* branch, yields `false`.

`Ne` is `xori(eq, true)`. The TaggedValue ↔ TaggedValue case
falls through to the existing path (and traps when one side is
non-Number) — that is `LIC-2.6c-tag-hetero-eq-1` from ADR 0064
and is intentionally left to the next sub-phase.

### ADR 0061 / 0063 plain-read-trap claims are superseded

ADR 0061's "plain `print(t[oob])` keeps trapping — separate
code path" and ADR 0063's `regression_inline_index_still_traps`
both rested on the inline read being Number-only. Once the
inline path goes through tagged dispatch, those claims become
wrong. Eight existing regression tests are reframed to expect
`"nil"` output (Lua-correct). Two new tests assert that
**arithmetic** on a Nil-tagged value still traps — the trap path
moves from the *read* to the *use*, where Lua's "nil + 1
errors" semantics actually require it.

### CA invariants preserved

| Layer    | Change                                                                                |
|----------|---------------------------------------------------------------------------------------|
| Lexer    | None                                                                                  |
| Parser   | None                                                                                  |
| AST      | None                                                                                  |
| HIR      | One predicate flip in `lower_expr::BinOp Eq/Ne` fold guard.                          |
| Codegen  | `Builtin::Print` arm Index special-case; `emit_tagged_eq_runtime_dispatch` helper called from `BinOp Eq/Ne`. |

## TDD Process

1. **Step 1 — Red.** 11 e2e tests in
   `tests/phase2_6c_tag_hetero_fix.rs`. 9 fail outright (Issue
   1 + Issue 2 cases), 2 already pass.
2. **Step 2a — Issue 1 Green.** Print-side dispatch — 7 of the
   11 hetero-fix tests pass. Reframe 8 existing regression
   tests that assumed inline trap.
3. **Step 2b — Issue 2 Green.** HIR fold guard +
   `emit_tagged_eq_runtime_dispatch`. All 11 hetero-fix tests
   pass; total green at **794** (= 781 + 11 new + 2 new
   arith-trap regressions; the 8 reframed tests stay in count).
4. **Step 3 — ADR + AGENTS + commit.**

## Alternatives Considered

- **Make `HirExprKind::Index` itself non-trapping.** Forces every
  consumer (arith, cmp, tostring, type, …) to handle a tagged
  return type. Blast radius spans nearly every emit_expr arm.
  Rejected — too invasive for a fix sub-phase.
- **HIR-reject `TaggedValue == X`** when X is not Nil. Avoids
  silent miscompile but pushes a `tostring(x) == "..."`
  workaround onto users. Rejected — the runtime dispatch is
  cheap and matches Lua semantics directly.
- **Eager local widening.** Materialise *every* `Index` into
  a fresh local kind so all consumers see TaggedValue. Same
  blast radius as making `Index` non-trapping. Rejected.
- **Combine print fix and eq fix into separate ADRs.** Both are
  P1 from the same review and share the "TaggedValue read
  surfaces wider than the original ADR 0064 covered" framing.
  Single ADR keeps the rationale together.

## Consequences

- HIR: ~10 LOC. Codegen: ~250 LOC (Index special-case +
  runtime dispatch + helper).
- 11 new e2e tests in `tests/phase2_6c_tag_hetero_fix.rs`. 8
  existing regression tests reframed to "prints nil". 2 new
  arith-trap tests (one in `phase2_6a_arr_array_index.rs`, one
  in `phase2_6c_tag_arr_holes.rs`) to assert the trap *does*
  still fire on `nil + 1`-style use. **Total green at 794.**
- **`LIC-2.6c-tag-hetero-inline-1` (new): resolved.** Inline
  table reads in print position now match Lua spec.
- **`LIC-2.6c-tag-hetero-eq-1`: partial → mostly resolved.**
  Local-vs-literal comparisons match runtime tag; Local-vs-Local
  still falls through to the trapping path and is left for the
  next sub-phase.
- ADR 0061 / 0063's plain-read-trap statements are explicitly
  superseded. Their reframed tests reference this ADR.
- Other consumers of `Index` (`tostring`, `type`, arith / cmp
  outside of print-arg position) still trap on non-Number tag.
  Those surfaces are smaller and rarer than `print` and stay as
  follow-up work.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0064.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `print(t[oob])` prints nil; non-print uses still extract f64 | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number/Bool/String/Nil supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number/Bool/String supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline print: nil; non-print uses still trap on extract | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number/Bool/String/Nil-delete supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | resolved (ADR 0062) |
| LIC-2.6c-tag-locals-1 | `type(x)` for widened local | runtime dispatch on actual tag | static "number" | new (ADR 0063) |
| LIC-2.6c-tag-hetero-fn-tbl-1 | Function/Table table values | accepted | rejected at HIR | new (ADR 0064) |
| LIC-2.6c-tag-hetero-eq-1 | `==`/`~=` between two `TaggedValue` locals | runtime tag-aware | trap when one side is non-Number | partial → **mostly resolved (this ADR; Local-Literal works, Local-Local still pending)** |
| LIC-2.6c-tag-hetero-inline-1 | inline `print(t[k])` for hetero values | prints typed value or "nil" | runtime tag dispatch | **resolved (this ADR)** |

## Out of Scope

- **Local-vs-Local TaggedValue equality** — needs a 2-tag
  cross-product dispatch. Left for `LIC-2.6c-tag-hetero-eq-1`'s
  follow-up sub-phase.
- **Other consumers of `Index` (arith, cmp, tostring, type,
  builtins)** for hetero values — would extend the same Index
  → tmp tagged slot pattern. Left until profiling / actual user
  reports show it matters more than the print site did.
- **Function-return widening** — `local x = f()` where `f`
  returns nil/heterogeneous.
- **Function/Table values in tables** — needs ucast / cycle /
  closure-escape work.
- **`type(x)` runtime dispatch on widened locals** — still
  static (LIC-2.6c-tag-locals-1).
