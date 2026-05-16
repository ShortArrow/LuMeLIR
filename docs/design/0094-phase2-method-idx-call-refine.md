# 0094. Phase 2.6+-method-idx-call-refine: Index-Callee Call Arg Refinement + Helper Extract

- **Status:** Accepted
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091 → ADR 0092 → ADR 0093 chain. ADR 0093 closed MethodCall arg
refinement; this ADR closes the orthogonal Index-callee Call
refinement carry-over identified explicitly in ADR 0093 Non-goals:

```lua
local t = {}
function t.helper(name) return "hello " .. name end
print(t.helper("world"))
-- Day 0: IndirectCallNoCandidates { param_kinds: [String], ret_kinds: [Number] }
```

`function t.helper(name)` was registered into
`method_funcs[("t", "helper")]` by ADR 0093 Pass 1. But the
chunk-walker's `Call` arm only fires refinement when `callee` is
`Ident`. For `t.helper(...)`, callee is
`Index { target: Ident("t"), key: Str("helper") }` — refinement
skipped, `name` stayed Number, dispatch mismatched String.

Codex post-ADR-0093 review (6 視点) verdict: **Refactor → Go**.
Critical: extract `try_refine_func_args` helper so the three
refinement arms (Ident-Call, MethodCall, Index-callee Call) share the
kinds/seen update body. The differences are just FuncId lookup +
arg base index.

## Non-goals (top-of-ADR)

- **Index target / key non-literal** — `(get_obj()).m(x)`,
  `t[k](x)`. Safely skipped via lookup miss.
- **Function-kind upvalue refinement** — orthogonal; separate ADR.
- **Source-order shadowing resolution** — same ADR 0091+ chain
  problem; future ADR addresses all three indices.
- **`self` kind refinement** — stays Table per ADR 0092 MVP. For
  colon-def + explicit-self call `t.m(t, x)`, args=[t, x] refines
  kinds[idx][0]=Table (from t) — but `lower_method_def` re-seeds
  external_kinds[0]=Table at the for_function call site, making
  the index-0 refinement a no-op (Table → Table).
- **Multi-call-site param-kind merge** — first-wins.
- **Codegen / parser / lexer / CLI changes** — zero-diff (CA
  invariant).

## Context

Today's `infer_user_function_param_kinds` (`src/hir/mod.rs`):

```
visit_expr arm Call:
  if callee = Ident(name) AND name in function_names → refine (base=0)

visit_expr arm MethodCall:
  if receiver = Ident AND (recv, method) in method_funcs → refine (base=1)
```

After this ADR:

```
visit_expr arm Call:
  if callee = Ident(name) AND name in function_names → try_refine_func_args(idx, 0)
  if callee = Index { target: Ident, key: Str }
        AND (target, key) in method_funcs              → try_refine_func_args(idx, 0)

visit_expr arm MethodCall:
  if receiver = Ident AND (recv, method) in method_funcs → try_refine_func_args(idx, 1)
```

The `try_refine_func_args(idx, base, args, kinds, seen)` helper
captures the common body (arity check, kinds/seen update). Pure
side-effect on slices.

## New surface

- **`try_refine_func_args`** in `src/hir/mod.rs` (nested inside
  `infer_user_function_param_kinds`). Pure update helper:
  ```rust
  fn try_refine_func_args(
      idx: usize,
      base: usize,
      args: &[Expr],
      kinds: &mut [Vec<ValueKind>],
      seen: &mut [bool],
  ) {
      if !seen[idx] && args.len() + base == kinds[idx].len() {
          for (i, a) in args.iter().enumerate() {
              kinds[idx][i + base] = ast_arg_kind(a);
          }
          seen[idx] = true;
      }
  }
  ```
- **Index-callee refinement secondary if-let** inside the existing
  `Call` arm — lookup `(target_ident, key_str)` in `method_funcs`,
  refine via the helper with base=0.

## Reuse

- `method_funcs` index (ADR 0093) — no change.
- `function_names` (existing) — no change.
- ADR 0093's `register_method_signature` Pass 1 walk — no change.
- ADR 0093's `lower_method_def` `external_kinds` plumbing — no change
  (refined kinds flow through `functions[id.0].params` as before).
- `ast_arg_kind` helper (existing).

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: helper extract eliminates
  duplication across three arms; future fixes touch one place.
- [x] **#2 TDD**: 3 e2e per failure surface (dotted Index-callee
  / colon explicit-self / ADR 0093 regression).
- [x] **#3 FP**: pure pre-pass preserved; helper is a pure
  side-effect on slices.
- [x] **#4 CA**: HIR-internal; `method_funcs` reuse only.
- [x] **#5 Security**: Ident+Str pattern restriction safely skips
  unrecognized shapes; same shadowing semantics as ADR 0091/0092/0093.
- [x] **#6 Documentation**: ADR 0094 this file; tagged-semantics §8;
  AGENTS.md ‣ 2.6+-method-idx-call-refine.

## Test count delta

```
Step 0:  1035 → 1036 (2 Red + 1 always-green regression-pin)
Step 1:  1035 → 1036 (helper extract; regression-pin still green;
                       2 still Red — Index-callee not yet routed)
Step 2:  1035 → 1038 (Index-callee refinement; 2 Red flip Green)
Step 3:  1035 → 1038 (clippy + fmt only)
Step 4:  1035 → 1038 (docs only)

Final: 1035 → 1038 green, single atomic commit
  feat(hir,docs): Index-callee Call arg refinement + helper extract (ADR 0094)
```

## Verification

- `cargo test --no-fail-fast` → **1035 → 1038**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local t = {}
  function t.helper(name) return "hello " .. name end
  print(t.helper("world"))' > /tmp/x.lua
  cargo run --quiet -- compile /tmp/x.lua && /tmp/x   # → hello world
  ```

## Future work

- **Index target with non-Ident root** — `(get_obj()).m(x)`,
  `t[k](x)`. Needs deeper receiver-name walker.
- **Index-callee refinement when callee resolves via name rebind** —
  `local g = t; g.m(x)`. Needs binding-aware index.
- **Source-order shadowing resolution** — same ADR 0091+ chain
  problem; future ADR addresses both `function_names` and
  `method_funcs`.
- **`self` refinement** — once dispatcher gains arg widening.

## ADR number / phase tag

ADR 0094 = Index-Callee Call Arg Refinement + Helper Extract.
Phase tag: `2.6+-method-idx-call-refine` under existing
`2.6+ tables / metatables` sub-lane. Builds on ADR 0091 + 0092 +
0093; closes the explicit ADR 0093 Non-goals carry-over for the
Index-callee Call path.
