# 0096. Phase 2.6+-multi-segment-method-def: Multi-Segment Method-Def Parser Delta

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0092 (Method colon syntax, 2026-05-16) landed single-segment
method-def. The non-goal "multi-segment method-def" was a tracked
carry-over. ADR 0095 (`bd13c70`, 2026-05-16) landed nested
IndexAssign/Index target widening with TAG_TABLE runtime narrow —
the chokepoint prerequisite for nested table writes.

This ADR closes the ADR 0092 carry-over: `parse_method_def` now
accepts multi-segment receivers (`function a.b.c.m()` /
`function a.b.c:m()`) and HIR folds the receiver chain into a
nested Index AST, reusing ADR 0095's widen + codegen TAG_TABLE
narrow unmodified.

Codex review (6 視点) verdict: **Refactor → Go**. Critical fix:
FuncId allocation must happen for ALL MethodDef (any segment
count); the `method_funcs` index limitation only governs call-site
refinement, not FuncId source-of-truth.

## Non-goals (top-of-ADR)

- **Multi-segment method-call refinement** — `a.b.c.m(x)` call-site
  refinement via chunk-walker. ADR 0091/0093/0094 walker matches
  `Index{Ident, Str}` shape only; nested Index chains skip.
  Future ADR.
- **Multi-segment colon-call** — `a.b.c:m(...)` requires MethodCall
  with Index receiver (ADR 0092 ComplexMethodReceiver MVP boundary).
  Future ADR can extend the MethodCall receiver shape; today's
  workaround is the explicit-self form via Ident-bound aliases.
- **Source-order shadowing resolution** — orthogonal carry-over
  shared with ADR 0091-0094.
- **Bare top-level `function NAME() end`** — requires globals.
- **`self` widen to TaggedValue** — ADR 0092 MVP policy preserved.
- **Multi-segment method-def with non-Ident segments** —
  `function get_obj().f() end`. Out of MVP; future ADR.
- **Codegen changes** — `emit.rs` zero-diff. ADR 0095 already
  provides the runtime narrow infrastructure.

## Context

ADR 0092 `parse_method_def` consumed exactly one `.IDENT` or
`:IDENT` separator after the head Ident:

```
parse_method_def:
  bump 'function'
  consume Ident (receiver)
  consume Dot or Colon → is_colon set
  consume Ident (method)
  parse signature+body
  emit MethodDef { receiver: String, method, is_colon, params, body }
```

3+ segments yielded `UnexpectedToken { Dot }` or `UnexpectedToken { Colon }`
at the SECOND separator.

This ADR extends the parser loop, the AST shape, and the HIR
plumbing while keeping `lower_method_def` as a single chokepoint
that folds receiver_chain into Ident/Index AST and routes through
ADR 0095's widen.

## Reframing

After this ADR:

```
parse_method_def:
  bump 'function'
  consume Ident → segments = [first]
  loop:
    peek == Dot:    consume Dot+Ident → segments.push(ident)
    peek == Colon:  consume Colon+Ident → segments.push(method); is_colon=true; break
    peek == LParen: break (dotted form; last segments entry is method)
    else: error UnexpectedToken
  if segments.len() < 2: error UnexpectedToken { LParen } (bare global; ADR 0092 pin)
  method = segments.pop()
  receiver_chain = segments
  parse signature+body
  emit MethodDef { receiver_chain: Vec<String>, method, is_colon, params, body }
```

HIR `lower_method_def` (single chokepoint, no segment-count branch):
```
fold receiver_chain into target_ast (Ident → Index → Index ... → Index)
lower_expr(target_ast) → HirExpr
widen_index_for_assign_target(target_hir) → IndexTagged when chain.len() ≥ 2
target_kind ∈ {Table, TaggedValue} → OK
emit IndexAssign(target_hir, Str(method), FunctionRef(funcid))
```

Codex critical fix:
- `alloc_method_signature` (formerly `register_method_signature`) is
  now alloc-only: pushes HirFunction placeholder, returns FuncId.
  No `method_funcs` insertion.
- Pass-1 loop calls `alloc_method_signature` for EVERY MethodDef and
  appends the FuncId to `methoddef_func_ids: Vec<FuncId>`
  (source-ordered).
- Pass-1 loop conditionally inserts into `method_funcs[(receiver_chain[0], method)]`
  ONLY when `receiver_chain.len() == 1` (call-site refinement
  boundary; ADR 0093/0094 walker matches single-Ident receivers).
- `lower_method_def` consumes FuncId via `methoddef_func_ids[methoddef_seq]`
  + post-increment (mirrors `funcdef_seq` for FunctionDef).

## New surface

- **AST** (`src/parser/ast.rs`): `StmtKind::MethodDef.receiver: String`
  renamed to `receiver_chain: Vec<String>`. Length-1 is the single-
  segment ADR 0092 path.
- **Parser** (`src/parser/mod.rs`):
  - `parse_method_def` loop over `.IDENT` and final `:IDENT` /
    LParen sentinel.
  - `strip_span_stmt` MethodDef destructure updated.
- **HIR** (`src/hir/mod.rs`):
  - `alloc_method_signature` helper (formerly `register_method_signature`):
    alloc-only, no `method_funcs` insertion.
  - Pass-1 walks ALL MethodDef; conditional `method_funcs` insertion
    for `receiver_chain.len() == 1`; appends to
    `methoddef_func_ids: Vec<FuncId>`.
  - `LowerCtx::methoddef_func_ids: Vec<FuncId>` + `methoddef_seq: usize`
    threaded through `new` / `for_function` / `lower_into_function`.
  - `lower_method_def`:
    - Signature: `receiver: &str` → `receiver_chain: &[String]`.
    - FuncId source: `self.methoddef_func_ids[self.methoddef_seq]`
      + post-increment.
    - Fold receiver_chain into target_ast (Ident → Index chain).
    - Apply ADR 0095 `widen_index_for_assign_target` after
      `lower_expr` so nested targets widen to TaggedValue (length-1
      stays Ident, no widen).
    - Accept Table OR TaggedValue target_kind.

## Reuse

- ADR 0095 `widen_index_for_assign_target` (`src/hir/mod.rs`) —
  applies automatically when chain length ≥ 2.
- ADR 0095 `emit_resolve_table_target_ptr` +
  `emit_narrow_indextagged_to_table_ptr` (`src/codegen/emit.rs`) —
  consumed unchanged.
- `register_function_signature` shape — pattern reused for the
  refactored `alloc_method_signature`.
- `funcdef_seq` pattern (`src/hir/mod.rs`) — template for
  `methoddef_seq`.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: `Vec<String>` chain unified;
  length-1 is the single-segment ADR 0092 path; single chokepoint
  at `lower_method_def`.
- [x] **#2 TDD**: 3 happy (dotted-3seg / colon-3seg compile-only /
  4seg-boundary) + 1 regression-pin (2-seg ADR 0092 path).
- [x] **#3 FP**: receiver_chain → nested Index AST is pure
  transform; fold-based construction in one chokepoint.
- [x] **#4 CA**: parser syntax-side; HIR normalizes; codegen
  zero-diff (`emit.rs` unchanged).
- [x] **#5 Security (Codex critical)**: FuncId allocation always
  happens; `method_funcs` registration is the conditional part.
  `methoddef_func_ids` + `methoddef_seq` mirrors `funcdef_seq`
  (proven pattern).
- [x] **#6 Documentation**: ADR 0096 this file; tagged-semantics §8;
  AGENTS.md `‣ 2.6+-multi-segment-method-def` row.

## Test count delta

```
Step 0:  1042 → 1043 (3 Red + 1 always-green regression-pin)
Step 1+3-HIR:  1042 → 1043 (AST rename + HIR plumbing; tests
                              unchanged at Day-0 because parser
                              still rejects multi-segment)
Step 2:  1042 → 1046 (parser loop + colon test reshaped to compile-only;
                       3 Red → Green; ADR 0095 widen at lower_method_def
                       added to handle nested target kind)
Step 4:  1042 → 1046 (clippy + fmt; codegen zero-diff)
Step 5:  1042 → 1046 (docs only)

Final: 1042 → 1046 green, single atomic commit
  feat(parser,hir,docs): multi-segment method-def parser delta (ADR 0096)
```

## Verification

- `cargo test --no-fail-fast` → **1042 → 1046**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local app = {}
  app.utils = {}
  function app.utils.format(x) return x + 1 end
  print(app.utils.format(41))' > /tmp/m.lua
  cargo run --quiet -- compile /tmp/m.lua && /tmp/m   # → 42
  ```
  Number args dispatch correctly via ADR 0091/0094 refinement; non-
  Number arg variants need ADR 0091-0094 carry-over future work for
  multi-segment receiver refinement.

## Future work

- **Multi-segment method-call** — `a.b.c.m(x)` call-site (refinement
  + receiver kind narrowing). Requires chunk-walker extension to
  match nested Index callees AND dispatcher arg-widening or kind-
  narrowing for Table receivers.
- **Multi-segment colon-call** — `a.b.c:m(x)` would extend ADR 0092
  MethodCall to accept Index receivers (materialize_to_synth_local
  reuse).
- **Source-order shadowing resolution** — orthogonal ADR 0091+
  carry-over.
- **`self` widen to TaggedValue** — once dispatcher gains arg
  widening.
- **Multi-segment method-def with non-Ident segments** —
  `function get_obj().f() end`. Future ADR.

## ADR number / phase tag

ADR 0096 = Multi-Segment Method-Def Parser Delta. Phase tag:
`2.6+-multi-segment-method-def` under existing `2.6+ tables /
metatables` sub-lane. Closes ADR 0092's tracked carry-over; reuses
ADR 0095's HIR/codegen unmodified.
