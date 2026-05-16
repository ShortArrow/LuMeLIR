# 0093. Phase 2.6+-method-arg-refine: MethodCall Arg Refinement via Pass-1 MethodDef Registration

- **Status:** Accepted
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091 (`82e6de9`) → ADR 0092 (`d1eac46`) chain. ADR 0092's manual
smoke surfaced an explicit carry-over:

```lua
local obj = {}
function obj:greet(name) return "hello " .. name end
print(obj:greet("world"))
-- IndirectCallNoCandidates { param_kinds: [Table, String], ret_kinds: [Number] }
```

`function obj:greet(name)` registered a method with
`external_kinds = [Table, Number]` (Table self per ADR 0092 MVP,
Number default for `name`). The chunk-walker
`infer_user_function_param_kinds` walked only `Call { callee: Ident }`
for refinement; ADR 0092 Step 3 gave `MethodCall` a descend-only arm
(intentional carry-over). So `name` stayed Number, dispatch
mismatched String, and `obj:greet("world")` failed.

ADR 0093 closes this carry-over without changing dispatch semantics.

## Non-goals (top-of-ADR per codex planning guideline #2)

- **Index-receiver MethodCall refinement** — `(obj.field):m(x)`,
  `t[1]:m(x)`. Needs an Index-callee receiver-name walker; future
  ADR.
- **`self` kind refinement** — stays Table per ADR 0092 policy. The
  Pass-1.5 refinement intentionally skips index 0 (self); the
  `lower_method_def` site re-seeds Table on the `external_kinds`
  vector regardless.
- **Source-order shadowing resolution** — `(receiver, method)`
  collisions overwrite in the index (last-wins), same semantics as
  FunctionDef's `function_names`. Documented as carry-over; future
  ADR addresses both.
- **Multi-segment method-def** — ADR 0092 carry-over.
- **`infer_user_function_param_kinds` refinement via Ident-callee
  Call for Index-callee paths** — orthogonal ADR 0091 carry-over;
  `t.m(arg)` direct call still doesn't refine `m`'s param kinds.
- **Param-kind merge across multiple call sites with different arg
  kinds** — first-call-site-wins (same as FunctionDef); future ADR
  could merge to TaggedValue when conflicting.

## Context

The pass order in `lower()` (see `src/hir/mod.rs:996-1097` before
this ADR; equivalent block after):

```
Pass 1   :  walk chunk → register FunctionDef in function_names (FuncId alloc)
Pass 1.5 :  infer_user_function_param_kinds(chunk, function_names) → refine params
Pass 2   :  source-order lowering; FunctionDef bodies via lower_into_function
              MethodDef lowering at this point: ALLOCATES FuncId here
```

The Codex post-ADR-0092 critical: at Pass 1.5, MethodDef's FuncId
doesn't exist. The chunk-walker sees MethodDef + MethodCall AST nodes
but can't connect them by FuncId because the alloc was deferred to
Pass 2.

## Reframing

After this ADR:

```
Pass 1   :  walk chunk
              FunctionDef → function_names: HashMap<String, FuncId>
              MethodDef   → method_funcs:   HashMap<(String, String), FuncId>
Pass 1.5 :  infer_user_function_param_kinds(chunk, function_names, method_funcs)
              MethodCall arm refines via method_funcs lookup; args index 1..N
Pass 2   :  source-order lowering
              MethodDef lowering USES pre-allocated FuncId (no alloc here)
```

`Callee::IndirectDispatch` unchanged. `emit.rs` unchanged. CA
invariant preserved.

## New surface

- **`register_method_signature`** in `src/hir/mod.rs` — pass-1
  registration helper mirroring `register_function_signature`.
  Pushes `HirFunction { name = "", mangled_name = "user_anon_<idx>" }`
  with `effective_params` (post-self-prepend) and default Number
  param kinds; inserts `(receiver, method) -> FuncId` into the
  shared index.
- **`LowerCtx::method_funcs`** field — mirror of `function_names`
  for MethodDef-registered FuncIds. Threaded through `new`,
  `for_function`, and `lower_into_function`.
- **`infer_user_function_param_kinds` signature extension** —
  takes `method_funcs: &HashMap<(String, String), FuncId>` and adds
  a `MethodCall` arm that refines via lookup. Ident receiver
  required for static FuncId resolution; explicit args (index 1..N)
  refine from literal kinds.
- **Pass-1 loop in `lower()`** — walks `StmtKind::MethodDef` and
  calls `register_method_signature` for each, building
  `method_funcs`. Sequential to the FunctionDef walk so the
  `funcdef_seq` counter at Pass 2 still maps 1:1 onto FunctionDef
  FuncIds in source order.
- **`lower_method_def` switch** — replaces the inline FuncId alloc
  + placeholder push with `let id = self.method_funcs[&(recv, method)]`.
  The `external_kinds` vector now reads `self.functions[id.0].params`
  (carrying Pass-1.5 refinement) instead of a fresh
  `vec![Number; n]`. Self at index 0 is re-seeded to Table
  unconditionally per ADR 0092 policy.

## Reuse

- `register_function_signature` shape (`src/hir/mod.rs`) — template
  for `register_method_signature`.
- `function_names` parallel — `method_funcs` carries the same
  semantics (name → FuncId index, last-wins on collision).
- ADR 0092's `lower_method_def` — modified in-place; FuncId alloc
  block deleted, lookup substituted.
- `ast_arg_kind` helper — existing literal-kind extraction.
- ADR 0091's `pending_pre_stmts` + `materialize_to_synth_local`
  unchanged.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: `register_method_signature`
  mirrors `register_function_signature` exactly; no new module,
  same pass-1 / pass-2 pattern. Future Index-receiver extension
  reuses the `method_funcs` index pattern.
- [x] **#2 TDD**: 4 e2e per arg-kind face (String / Bool / multi-arg
  String / FunctionDef regression-pin). Failure surface is
  consistently `IndirectCallNoCandidates` at Day 0 (per Codex's
  prediction).
- [x] **#3 FP**: pure pre-pass (Pass 1 loop) builds immutable index;
  Pass 1.5 reads only.
- [x] **#4 CA**: MethodDef registration at 1 chokepoint (Pass 1
  loop); refinement at 1 chokepoint (visitor `MethodCall` arm);
  pre-allocated FuncId consumed at 1 chokepoint (`lower_method_def`).
- [x] **#5 Security (shadowing)**: same `last-wins` behavior as
  FunctionDef's `function_names`. Documented as carry-over.
- [x] **#6 Documentation**: ADR 0093 this file; tagged-semantics.md
  §8 row; AGENTS.md ‣ 2.6+-method-arg-refine row.

## Test count delta

```
Step 0:  1031 → 1032 (3 Red + 1 always-green regression-pin)
Step 1:  1031 → 1032 (extract helper; no caller; no test delta)
Step 2:  1031 → 1032 (Pass 1 wiring; tests still 3 Red; ADR 0092
                       regression all-green)
Step 3:  1031 → 1035 (refinement wired; 3 Red flip Green)
Step 4:  1031 → 1035 (clippy + fmt only; clippy::too_many_arguments
                       allow on for_function — internal helper)
Step 5:  1031 → 1035 (docs only)

Final: 1031 → 1035 green, single atomic commit
  feat(hir,docs): MethodCall arg refinement via Pass-1 MethodDef registration (ADR 0093)
```

## Verification

- `cargo test --no-fail-fast` → **1031 → 1035**
- `cargo clippy --all-targets -- -D warnings` → clean (with
  `#[allow(clippy::too_many_arguments)]` on `for_function`, which
  now has 8 args after the `method_funcs` plumbing)
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local obj = {}
  function obj:greet(name) return "hello " .. name end
  print(obj:greet("world"))' > /tmp/r.lua
  cargo run --quiet -- compile /tmp/r.lua && /tmp/r   # → hello world
  ```
  Carry-over closed.

## Future work

- **Source-order shadowing resolution** — when `(receiver, method)`
  shadows, use the most-recent definition for refinement of
  preceding call sites only. Requires source-order walk tracking
  in-scope MethodDef at each MethodCall position. Same problem
  applies to FunctionDef; future ADR addresses both.
- **Index-receiver MethodCall refinement** — `(obj.field):m(x)`,
  `t[1]:m(x)`. Requires receiver-name walker over Index chains.
- **`self` refinement** — once dispatcher gains arg widening (ADR
  0092 Future work), `self` could be inferred TaggedValue and
  refined per call site.
- **Param-kind merge across multiple call sites** — today
  first-wins; future ADR could merge to TaggedValue when conflicting
  (Bool first, then String → TaggedValue param to match both).

## ADR number / phase tag

ADR 0093 = MethodCall Arg Refinement via Pass-1 MethodDef
Registration. Phase tag: `2.6+-method-arg-refine` under existing
`2.6+ tables / metatables` sub-lane. Builds on ADR 0091 + 0092;
closes their explicit carry-over.
