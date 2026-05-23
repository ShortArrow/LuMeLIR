# 0097. Phase 2.6+-multi-seg-call-refine: Multi-Segment Method-Call Refinement via Chain-Keyed method_funcs

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091/0093/0094/0096 chain. ADR 0096 closed the multi-segment
method-DEF parser delta (`function a.b.c.m() end` parses + lowers +
emits correct codegen). The corresponding CALL-side refinement was
deferred and tracked as a collective carry-over in ADR 0091/0094/0096
"Future work" sections.

This ADR closes that carry-over: `app.utils.format("world")` now
refines `format`'s `name` param to String at the call site, dispatch
matches, end-to-end runtime works.

Codex post-ADR-0096 review (6 視点) verdict: **Refactor → Go**.
Critical: unify `method_funcs` to chain-keyed
`HashMap<(Vec<String>, String), FuncId>`. Single-segment uses
length-1 chain key. Avoid maintaining two indices (single-seg
String key + multi-seg Vec key) — that creates lookup-rule
duplication compounding with future colon-multi-seg / receiver-
narrowing / shadowing work.

## Non-goals (top-of-ADR per codex guideline)

- **Multi-segment colon-call** — `a.b.c:m(x)` MethodCall with Index
  receiver. ADR 0092's ComplexMethodReceiver MVP boundary requires
  receiver=Ident at the call site. Future ADR loosens.
- **Receiver kind narrowing for explicit-self form** —
  `a.b.c.scale(a.b.c, x)` requires the receiver arg to be narrowed
  to Table at the call site. Dispatcher's strict-equal rejects.
  Future ADR pairs dispatcher widening with receiver narrowing.
- **Source-order shadowing resolution** — `method_funcs` keys
  remain last-wins (same as ADR 0091/0094 / `function_names`).
- **`self` widen to TaggedValue** — ADR 0092 MVP preserved.
- **Non-Ident chain head** — `(get_obj()).field.m(x)` skipped via
  `extract_index_chain` returning None (safe). Walker descends
  into args regardless.
- **Codegen changes** — `emit.rs` zero-diff (HIR-only refinement).

## Context

Pre-ADR-0097 state:

```
method_funcs: HashMap<(String, String), FuncId>

Pass-1 walk:
  if receiver_chain.len() == 1:
    insert((receiver_chain[0], method), FuncId)
  // Multi-segment MethodDefs registered FuncId via methoddef_func_ids
  // but NOT in method_funcs (gated to single-seg per ADR 0096).

Pass-1.5 walker Call arm (ADR 0094):
  if callee = Index { target: Ident, key: Str }:
    if method_funcs[(target_name, key_str)] exists:
      try_refine_func_args(idx, 0, ...)
```

Limitations:
- Multi-segment MethodDefs unreachable from refinement walker
  because they aren't in `method_funcs`.
- Walker pattern `Index { Ident, Str }` doesn't match nested
  Index chains.

Post-ADR-0097 state:

```
method_funcs: HashMap<(Vec<String>, String), FuncId>   // chain-keyed

Pass-1 walk:
  insert((receiver_chain.clone(), method), FuncId)
  // ALL MethodDef enter the index. Single-seg is length-1 chain.

Pass-1.5 walker Call arm:
  if extract_index_chain(callee) = Some((chain, method)):
    if method_funcs[(chain, method)] exists:
      try_refine_func_args(idx, 0, ...)

extract_index_chain (pure):
  Walks Index{Index{...{Ident, Str}...}, Str} chains.
  Returns None on non-Ident head OR non-Str key.
```

For single-segment: `t.helper("world")` callee =
`Index{Ident("t"), Str("helper")}`. `extract_index_chain` returns
`Some((["t"], "helper"))`. `method_funcs[(["t"], "helper")]` hits
the same FuncId previously stored under `("t", "helper")`. Path
unified.

For multi-segment: `app.utils.format("world")` callee =
`Index{Index{Ident("app"), Str("utils")}, Str("format")}`.
`extract_index_chain` returns `Some((["app", "utils"], "format"))`.
`method_funcs[(["app", "utils"], "format")]` hits the FuncId
registered by ADR 0096 Pass-1 for the multi-segment method-def.
Refinement fires.

## New surface

- **`method_funcs` type change** (`src/hir/mod.rs`):
  `HashMap<(String, String), FuncId>` →
  `HashMap<(Vec<String>, String), FuncId>`. 8 declaration sites
  updated via mechanical type swap.
- **`extract_index_chain` pure helper** (`src/hir/mod.rs`, ~50 LOC):
  Recursive walker; returns `Option<(Vec<String>, String)>`.
- **Pass-1 walk** (`src/hir/mod.rs:lower`):
  - Drop `if receiver_chain.len() == 1` gate.
  - Insert `(receiver_chain.clone(), method.clone()) -> FuncId` for
    ALL MethodDef.
- **`infer_user_function_param_kinds` Call arm**: replaced
  single-segment if-let with `extract_index_chain` + chain-keyed
  lookup. MethodCall arm gets length-1 wrap for the single-Ident
  receiver path.

## Reuse

- `try_refine_func_args` (ADR 0094 helper) — base=0 for the
  Index-callee refinement; unchanged.
- `methoddef_func_ids` Vec (ADR 0096) — unchanged Pass-2 consumer.
- `ast_arg_kind` (existing) — literal arg-kind extraction.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**:
  `method_funcs` unified to chain-keyed. One lookup rule, one
  source of truth.
- [x] **#2 TDD**: 2 happy (3-seg / 4-seg) + 1 regression-pin
  (single-seg refinement unchanged) per Codex's specific
  recommendation.
- [x] **#3 FP**: `extract_index_chain` pure walker; `try_refine_func_args`
  reuse; lookup + refine separated.
- [x] **#4 CA**: HIR-only; `src/codegen/`, `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: chain head non-Ident → None safe-skip.
  Shadowing same as `function_names` last-wins; documented in
  §Non-goals.
- [x] **#6 Documentation**: ADR 0097 captures collective-carry-over
  framing + Codex unification fix + HIR-only annotation.

## Test count delta

```
Step 0:  1046 → 1047 (2 Red + 1 always-green regression-pin)
Step 1+2: 1046 → 1047 (type change + always-insert; Reds persist
                        because walker doesn't extract chains yet)
Step 3:  1046 → 1049 (extract_index_chain + walker rewire; 2 Red Green)
Step 4:  1046 → 1049 (clippy + fmt)
Step 5:  1046 → 1049 (docs only)

Final: 1046 → 1049 green, single atomic commit
  feat(hir,docs): multi-segment method-call refinement via chain-keyed method_funcs (ADR 0097)
```

## Verification

- `cargo test --no-fail-fast` → **1046 → 1049**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local app = {}
  app.utils = {}
  function app.utils.format(name) return "hello " .. name end
  print(app.utils.format("world"))' > /tmp/c.lua
  cargo run --quiet -- compile /tmp/c.lua && /tmp/c   # → hello world
  ```

## Future work

- **Multi-segment colon-call** — `a.b.c:m(x)` MethodCall with Index
  receiver. Requires ADR 0092 ComplexMethodReceiver loosening +
  receiver materialize_to_synth_local for Index receivers (ADR 0091
  pattern extension).
- **Receiver kind narrowing for explicit-self form** — paired with
  dispatcher arg-widening; unblocks `a.b.c.scale(a.b.c, x)`.
- **Source-order shadowing resolution** — orthogonal carry-over.
- **`self` widen to TaggedValue** — once dispatcher gains arg
  widening.
- **Method-call refinement when callee resolves via name rebind** —
  `local g = a.b; g.m(x)` — needs binding-aware lookup.

## ADR number / phase tag

ADR 0097 = Multi-Segment Method-Call Refinement (Chain-Keyed
method_funcs Unification). Phase tag:
`2.6+-multi-seg-call-refine` under existing `2.6+ tables /
metatables` sub-lane. Closes the ADR 0091/0094/0096 collective
carry-over for the dotted multi-segment call path.
