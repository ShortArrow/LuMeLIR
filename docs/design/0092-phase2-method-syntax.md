# 0092. Phase 2.6+-methods: Method Colon Syntax Desugar over Index-Callee Calls

- **Status:** Accepted
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091 plan v1 (2026-05-10, "Method colon syntax") was aborted and
re-scoped as ADR 0091 v2 ("HIR Callee Normalization for Index-Callee
Calls"). ADR 0091 v2 landed at `82e6de9` on 2026-05-14, providing the
`pending_pre_stmts` hoisting infra + `classify_callee_form` +
`materialize_callee_to_local` (renamed by this ADR to
`materialize_to_synth_local`) that is the foundation methods always
implicitly required.

Codex post-0091 review (2026-05-16, 6 視点) confirmed Methods is the
natural follow-up, and identified **4 critical fixes** before plan
v1's framing could be reused safely:

1. **Don't write "sugar-only"** — Methods is partial-sugar; def-side
   + `self` policy + receiver-shape check are infra.
2. **`self` param-kind policy upfront** — must be decided before
   implementation, not discovered during.
3. **Method-def desugar at HIR chokepoint** — parser preserves AST
   source shape (`MethodDef` variant); HIR owns the `IndexAssign +
   FunctionRef` emission.
4. **Receiver-shape check explicit** — `ComplexMethodReceiver` rejects
   any receiver containing `Call`, `FunctionExpr`, `BinOp`, or
   `UnaryOp` (recursive walk).

A Plan-agent design review surfaced 5 additional depth concerns now
baked into this ADR: visitor-surface ripple, `self` plumbing into
`for_function`'s `external_kinds`, ADR 0083 hetero-return LIC
interaction, lexer single-char dispatch, and refactor of
`materialize_callee_to_local`.

## Non-goals (top-of-ADR per codex planning guideline #2)

- **Multi-segment method-def** (`function a.b.c:m() end`,
  `function a.b.c.m() end`). Single-segment Ident receiver only.
- **Method-def with non-Ident receiver** (`function (f()):m() end`).
- **Bare top-level `function NAME() end`** — requires globals (not
  yet supported). Rejected with `UnexpectedToken { actual: LParen }`.
- **Method-call with non-Ident receiver** (`(obj.field):m()`,
  `t[1]:m()`) — strict-equal param-kind dispatch (ADR 0082) does not
  accept the widened receiver kind; future ADR lifts once dispatch
  permits arg widening.
- **Hetero-return method bodies** — trip existing
  LIC-2.6c-tag-locals-fn-indirect-1 at `src/hir/mod.rs` IndexAssign
  function-value branch. Carry-over from prior ADRs; surfaces as
  `TypeMismatch`.
- **Metatables / `__call`**.
- **`infer_user_function_param_kinds` chunk-walker refinement for
  MethodCall args** — same status as ADR 0091's carry-over. User fns
  reached only via `obj:m(arg)` get default Number arg kinds beyond
  `self`. Workaround: include at least one direct `obj.m(obj, arg)`
  call site in the chunk to refine kinds.

## Context

ADR 0091 v2 (`82e6de9`, 2026-05-14) closed the HIR callable-boundary
gap. `obj.m(args)` now runs end-to-end via the
`pending_pre_stmts` hoisting infra + `Callee::IndirectDispatch`
(ADR 0082). The remaining user-visible gap is the method colon syntax:

```lua
local obj = {}
function obj:add(x) return x + 1 end   -- parser rejects (no Function-keyword stmt arm)
print(obj:add(41))                     -- lexer rejects ':' (no Colon token)
```

ADR 0092 closes both:

1. **Call-site** `recv:method(args)` — AST variant
   `ExprKind::MethodCall { receiver, method, args }` preserved
   through parser, desugared at HIR chokepoint to
   `Call(callee=Index(recv, Str(method)), args=[recv, ...args])`.
   Routes through ADR 0091 + 0082 dispatch chain.
2. **Method-def** `function recv:method(...) end` (and dotted
   `function recv.field(...) end`) — AST variant
   `StmtKind::MethodDef { receiver, method, is_colon, params, body }`
   preserved through parser, desugared at HIR chokepoint to
   `IndexAssign(recv, Str(method), FunctionRef)`. For colon form,
   `self` (kind `Table` — see "self param-kind policy" below) is
   prepended to params and plumbed via `for_function`'s
   `external_kinds` seam.

## Reframing (codex planning guideline #1: lowering chokepoint)

Methods is two syntactic constructs riding the same HIR chokepoint:

```
recv:method(args)         AST  → ExprKind::MethodCall { receiver, method, args }
                          HIR  → Call(Index(recv, Str(method)), [recv, ...args])
                                 then ADR 0091 IndexCallee → IndirectDispatch

function recv:m(p1, ...)  AST  → StmtKind::MethodDef { receiver, method, is_colon: true, params, body }
                          HIR  → IndexAssign(recv, Str(m), FunctionRef(synth_fid))
                                 effective_params = ["self", ...params]
                                 external_kinds[0] = Table (for is_colon)
```

3-layer split (mirrors ADR 0091 pattern):

| Layer | Module | Role |
|---|---|---|
| **Lexer** | `src/lexer/{mod,token}.rs` | `':'` → `TokenKind::Colon` single-char dispatch |
| **Parser** | `src/parser/{mod,ast}.rs` | `parse_call_suffix` Colon arm → `MethodCall`; `parse_stmt` Function arm (Ident-lookahead) → `MethodDef` |
| **HIR chokepoint** | `src/hir/mod.rs` | `lower_expr` MethodCall arm + `lower_method_def` for `MethodDef` |

`Callee::IndirectDispatch` is unchanged. `emit.rs` is unchanged. CA
invariant per ADR 0090 holds (`git diff --stat src/codegen/` = 0,
verified pre-merge).

## `self` param-kind policy

**Decision: `self` kind = `Table` for MVP.**

The Plan-agent originally recommended `TaggedValue` for future
metatables-compatibility. Implementation surfaced that ADR 0082's
strict-equal dispatcher rejects `(Table, Number)` call sigs against
`(TaggedValue, Number)` param kinds — `obj:add(41)` after desugar
passes `(obj:Table, 41:Number)` against a registered method with
`(self:TaggedValue, x:Number)` params, yielding
`IndirectCallNoCandidates`.

Two paths considered:
- **A.** Widen the receiver arg via always-materialize through a
  synthetic TaggedValue local. Hits a downstream `Index` lowering
  rejection (the synth callee's `Index` target wants Table-kind, but
  the TaggedValue synth local fails the check).
- **B.** Seed `external_kinds[0] = Table` so the registered method's
  param[0] matches Table receivers directly. Body `self.field` reads
  use the existing Index-over-Table path (kind Number per
  `infer_kind`).

Path B chosen. Future ADR (metatables / `__index`) lifts `self` to
TaggedValue once the dispatcher gains arg-widening or once an arity-
preserving widen-on-dispatch ABI is designed.

Plumbing: at `lower_method_def`,
`external_kinds = vec![ValueKind::Number; effective_params.len()]`
then `external_kinds[0] = Table` (only when `is_colon`). The merge
logic at `src/hir/mod.rs:1336-1339` (body_kinds vs external_kinds)
picks `external_kinds[i]` unless body usage upgrades to
`Function(_)` — semantically correct since a body that calls `self`
as `self()` IS using `self` as a function.

## Reuse (codex planning guideline #4: don't break safety boundaries)

- `materialize_callee_to_local` (ADR 0091, `src/hir/mod.rs`) **renamed
  to `materialize_to_synth_local`** accepting an arbitrary `&Expr`.
  One helper now serves both callee + receiver materialization. The
  ADR 0091 caller updates trivially to construct `Expr::Index { target,
  key }` inline before passing.
- `pending_pre_stmts` drain wrapper at `lower_stmt` (ADR 0091).
- `Callee::IndirectDispatch` chain (ADR 0082).
- `widen_index_for_local_init` (ADR 0063).
- `classify_callee_form` IndexCallee path (ADR 0091).
- `for_function`'s `external_kinds` seeding seam
  (`src/hir/mod.rs:1322-1342`).
- LIC-2.6c-tag-locals-fn-indirect-1 at IndexAssign function-value
  branch — automatically rejects hetero-return methods through the
  IndexAssign path; no new check needed.
- `parse_function_signature_and_body` (`src/parser/mod.rs:587`).
- `ParseError::UnexpectedToken` — no new parser-error variant.

## New surface

- **Lexer**:
  - `TokenKind::Colon`
  - `':' => Some(TokenKind::Colon)` single-char dispatch arm.
- **AST**:
  - `ExprKind::MethodCall { receiver: Box<Expr>, method: String, args: Vec<Expr> }`
  - `StmtKind::MethodDef { receiver: String, method: String, is_colon: bool, params: Vec<String>, body: Chunk }`
- **Parser**:
  - `parse_call_suffix` Colon arm — parses `:IDENT(args)` and emits MethodCall.
  - `parse_stmt` Function-keyword arm (with `Ident` lookahead so
    expression-position `function() ... end` keeps flowing through
    `parse_primary`'s FunctionExpr arm).
  - `parse_method_def` helper — parses receiver Ident, dispatches on
    Dot/Colon, parses signature/body, emits MethodDef. LParen at the
    sep position rejects as `UnexpectedToken { LParen, ... }`.
- **HIR error**:
  - `HirError::ComplexMethodReceiver { offset }` — typed error for
    rejected receiver shapes.
- **HIR lowering**:
  - `lower_expr` MethodCall arm — shape-check, Ident-required
    fast-path, desugar to Call+Index, recurse through `lower_call`.
  - `lower_method_def` — builds effective_params, seeds external_kinds,
    registers synth anon function, emits IndexAssign.
  - `check_method_receiver_shape` pure walker — recursive descent
    over `Index { target, key }`, rejects
    `Call/MethodCall/FunctionExpr/BinOp/UnaryOp`.
  - `MethodCall` arms added to `infer_param_kinds` and
    `infer_user_function_param_kinds` visitors (descend; refinement
    intentionally not extended).
  - `MethodDef` arms added to the same visitors (treat like
    FunctionDef — own scope, no outer descent).

## Test count delta

```
Step 0:  1024 → 1024 (6 Red + 1 always-Green regression-pin)
Step 1:  1024 → 1024 (lexer; Reds advance to ParseError)
Step 2:  1024 → 1025 (parser; bare_top_level_function_rejected Green)
Step 3:  1024 → 1025 (refactor; no test delta)
Step 4:  1024 → 1025 (HirError variant; no test delta)
Step 5:  1024 → 1028 (MethodCall HIR desugar; 2 happy + 1 typed-error Green)
Step 6:  1024 → 1031 (MethodDef HIR desugar; 2 happy + dotted/colon coverage Green)
Step 7:  1024 → 1031 (clippy + fmt only)
Step 8:  1024 → 1031 (docs only)

Final: 1024 → 1031 green, single atomic commit
  feat(lexer,parser,hir,docs): method colon syntax desugar (ADR 0092)
```

## Verification

- `cargo test --no-fail-fast` → **1024 → 1031**
- `cargo clippy --all-targets -- -D warnings` → clean (incidental
  Tidy-First on `lowered.into_iter()` patterns to satisfy
  clippy 0.1.95's `useless_conversion` lint)
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs` → **0**
  (CA invariant)
- Manual smoke (ADR 0090 `--emit hir`):
  ```bash
  echo 'local obj = {}
  function obj:add(x) return x + 1 end
  print(obj:add(41))' > /tmp/m.lua
  cargo run --quiet -- compile --emit hir /tmp/m.lua
  cargo run --quiet -- compile /tmp/m.lua && /tmp/m   # → 42
  ```
  Non-Number args require the carry-over workaround documented in
  Non-goals: include at least one direct `obj.m(obj, arg)` call site
  in the chunk to refine the method's first non-self param kind via
  `infer_user_function_param_kinds`. Without that, `obj:greet("world")`
  surfaces `IndirectCallNoCandidates { param_kinds: [Table, String] }`.

## Codex 4-critical-fix checklist

- [x] **#1 Don't write "sugar-only"**: ADR title is "Method Colon
  Syntax **Desugar** over Index-Callee Calls". Reframing section
  explicitly frames as "partial-sugar — def-side + self policy +
  receiver-shape check are infra."
- [x] **#2 `self` param-kind upfront**: `Table` chosen; plumbing
  via `for_function`'s `external_kinds[0]` documented in
  "self param-kind policy" section. Future TaggedValue lift noted.
- [x] **#3 HIR-chokepoint desugar**: AST keeps `MethodCall` /
  `MethodDef` variants (source-shape preserved); HIR owns
  `IndexAssign + FunctionRef` emission via `lower_method_def`. Parser
  does no desugar.
- [x] **#4 Receiver-shape check explicit**:
  `check_method_receiver_shape` pure walker; recursive descent over
  `Index { target, key }`; rejects
  `Call/MethodCall/FunctionExpr/BinOp/UnaryOp` as
  `ComplexMethodReceiver`.

## Future work

- **Multi-segment method-def** (`function a.b.c:m() end`). Walks
  Index chain at parser, emits chained IndexAssign-like desugar.
- **Bare top-level `function NAME() end`** — requires globals
  support. Likely Phase 3 (top-level scope rework).
- **`infer_user_function_param_kinds` extension for MethodCall args** —
  refines callee user-fn first non-self param based on call-site
  literal args.
- **`self` kind widen to TaggedValue** — once dispatcher gains arg-
  widening or arity-preserving widen-on-dispatch ABI lands.
  Prerequisite for non-Ident method-call receivers and for
  metatables.
- **Hetero-return methods** — requires lifting
  LIC-2.6c-tag-locals-fn-indirect-1; needs ABI-level arity
  reconstruction work.
- **Metatables + `__index` + `__call`** — Phase 3 territory.

## ADR number / phase tag

ADR 0092 = HIR Method Colon Syntax Desugar. Phase tag:
`2.6+-methods` under existing `2.6+ tables / metatables` sub-lane.
Builds on ADR 0091 (Index-callee Call normalization).
