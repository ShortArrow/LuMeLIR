# 0098. Phase 2.6+-name-rebind-refine: Top-Level Name-Rebind Refinement via Pass-1.5 alias_map

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0097 (`d2f0e33`, 2026-05-16) closed multi-segment method-call
refinement via chain-keyed `method_funcs`. ADR 0097's future-work
section flagged the remaining gap: name-rebind refinement.

```lua
local app = {}
app.utils = {}
function app.utils.format(name) return "hi " .. name end
local g = app.utils.format    -- rebind to local
g("world")                    -- today: IndirectCallNoCandidates
```

After widening at LocalInit (ADR 0063), `g` is a TaggedValue local.
`g("world")` is Ident-callee Call → IndirectDispatch. The walker
checks `function_names["g"]` (miss, g is local) and tries
`extract_index_chain(Ident(g))` (None, no Index chain). Refinement
skips; `format`'s `name` stays Number; String arg doesn't match.

Codex post-0097 review (6 視点) verdict: **Refactor → Go**. Critical
fix: use Pass-1.5 pure `alias_map` (chunk-walker builds it from
`StmtKind::Local` / `LocalMulti` AST). Don't extend `LocalInfo.func_id`
— that mixes pre-pass refinement facts with post-lowering metadata
and weakens CA.

## Non-goals (top-of-ADR)

- **Function-body rebind** — `function f() local g = a.b.method;
  g(x) end`. Pass-1 walk is TOP-LEVEL only; future ADR adds
  nested-scope alias_map.
- **Re-assignment alias** — `local g; g = a.b.method; g(x)`
  (Local + later Assign). Pre-pass only sees LocalInit shape.
- **Multi-step alias chains** — `local h = a.b.m; local g = h;
  g(x)`. Single-level only; future ADR adds fixed-point resolution.
- **Block-scoped shadowing** — `do local g = ... end; local g = ...;
  g(x)`. Last-wins at chunk-walker, same as `function_names` /
  `method_funcs` shadowing carry-over. Documented explicitly.
- **`local g = some_funcdef`** — already handled via ADR 0083
  `LocalInfo.func_id` (Function-kind locals). This ADR doesn't
  touch that path.
- **Codegen changes** — `emit.rs` zero-diff. Pure HIR Pass-1.5
  extension.

## Context

Today's refinement infrastructure (after ADR 0097):

```
function_names: HashMap<String, FuncId>                  // top-level FunctionDef
method_funcs:   HashMap<(Vec<String>, String), FuncId>   // chain-keyed MethodDef

infer_user_function_param_kinds Call arm:
  if callee = Ident(name) AND function_names[name] = Some(FuncId):
    try_refine_func_args(idx, 0, ...)
  if extract_index_chain(callee) = Some((chain, method))
        AND method_funcs[(chain, method)] = Some(FuncId):
    try_refine_func_args(idx, 0, ...)
```

This handles direct method calls (`a.b.m(x)`) but misses the
rebind pattern (`local g = a.b.m; g(x)`). The local g is widened
to TaggedValue at LocalInit; the call site is Ident-callee but
`g` isn't a top-level FunctionDef name.

## Reframing

After this ADR:

```
alias_map: HashMap<String, FuncId>                       // top-level Local rebinds

Pass-1 (extended after method_funcs builds):
  for stmt in chunk:
    StmtKind::Local { name, value }:
      if extract_index_chain(value) = Some((chain, method))
            AND method_funcs[(chain, method)] = Some(FuncId):
        alias_map.insert(name.clone(), FuncId)
    StmtKind::LocalMulti { names, values } when names.len() == values.len():
      for (n, v) in names.zip(values):
        ... same logic ...

infer_user_function_param_kinds Call arm (extended):
  ... existing function_names + method_funcs lookups ...
  if callee = Ident(name) AND !function_names.contains_key(name)
        AND alias_map[name] = Some(FuncId):
    try_refine_func_args(idx, 0, ...)
```

`alias_map` is built once in Pass-1 (post-`method_funcs`) and read
immutably in Pass-1.5. Pure AST-derived pre-pass refinement fact;
no mutation across pass boundaries.

For shadowing: `local g = a.f; ...; local g = b.g` produces two
inserts to `alias_map["g"]` — last-wins (HashMap insert).
Source-order overwrite, no block-scope tracking — same as
`function_names` / `method_funcs` shadowing carry-over.

## New surface

- **`alias_map` build loop** in `lower()` (`src/hir/mod.rs`, ~25 LOC):
  - Walk chunk top-level for `StmtKind::Local` and
    `StmtKind::LocalMulti`.
  - For each binding, apply `extract_index_chain(value)` (ADR 0097
    reuse).
  - On `method_funcs[(chain, method)]` hit, insert
    `(name, FuncId)`.
- **`infer_user_function_param_kinds` extension**:
  - New `alias_map: &HashMap<String, FuncId>` parameter threaded
    through `visit_stmt` / `visit_expr`.
  - Call arm: after `function_names` lookup, also try `alias_map`
    when callee is Ident and not in `function_names`.

## Reuse

- `extract_index_chain` (ADR 0097 helper) — pure AST → `(Vec<String>, String)`.
- `try_refine_func_args` (ADR 0094 helper) — base=0 for direct
  alias call.
- `method_funcs` (ADR 0097 chain-keyed) — read at Pass-1 alias_map
  construction.
- `function_names` (existing) — lookup priority for the Call arm.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: Pass-1.5
  pure `alias_map`, NOT `LocalInfo.func_id` extension.
- [x] **#2 TDD**: 4 tests — 2 happy + 1 regression + 1 negative pin
  (codex critical: separate failure surfaces).
- [x] **#3 FP**: pure walker builds; immutable read at Call arm.
- [x] **#4 CA**: HIR-only; `src/codegen/`, `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: chunk-level last-wins documented;
  source-order overwrite explicit in §Non-goals.
- [x] **#6 Documentation**: ADR 0098 + tagged-semantics §8 +
  AGENTS.md HIR-only / zero-diff annotation.

## Test count delta

```
Step 0:  1049 → 1050 (3 Red + 1 regression-pin always-green)
Step 1:  1049 → 1050 (alias_map built; walker doesn't read; Reds persist)
Step 2:  1049 → 1053 (Call arm extension; 3 Red → Green)
Step 3:  1049 → 1053 (clippy + fmt)
Step 4:  1049 → 1053 (docs only)

Final: 1049 → 1053 green, single atomic commit
  feat(hir,docs): top-level name-rebind refinement via alias_map (ADR 0098)
```

## Verification

- `cargo test --no-fail-fast` → **1049 → 1053**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local app = {}
  app.utils = {}
  function app.utils.format(name) return "hi " .. name end
  local g = app.utils.format
  print(g("world"))' > /tmp/r.lua
  cargo run --quiet -- compile /tmp/r.lua && /tmp/r   # → hi world
  ```

## Future work

- **Function-body rebind** — nested-scope `alias_map` for
  per-function-body rebinds.
- **Re-assignment alias** — `local g; g = a.b.m; g(x)`.
- **Multi-step alias chains** — fixed-point resolution of
  `local h = a.b.m; local g = h; g(x)`.
- **Method-call rebind** — `local g = a:m; g(x)`. Today, `a:m`
  is MethodCall AST (not Index chain); future ADR extends
  `extract_index_chain` or adds MethodCall arm to alias build.
- **Multi-segment colon-call** — same blocker chain as ADR 0097
  future-work.
- **Source-order shadowing resolution** — orthogonal ADR 0091+
  carry-over.

## ADR number / phase tag

ADR 0098 = Top-Level Name-Rebind Refinement via Pass-1.5
alias_map. Phase tag: `2.6+-name-rebind-refine` under existing
`2.6+ tables / metatables` sub-lane. Closes the ADR 0097
future-work for the top-level rebind case.
