# 0036. Phase 2.5f: Nested `local function` Definitions

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Through Phase 2.5e the lowering for `StmtKind::FunctionDef`
inside another function body was a deliberate `unimplemented!`.
Anonymous-function expressions (`local f = function() ... end`)
worked because they registered through the
`HirExprKind::FunctionExpr` path, but the named statement form
did not. Nested helpers — a normal Lua organisation pattern —
were therefore impossible to write:

```lua
local function fib_outer(x)
  local function fib(n)               -- panicked at HIR-time
    if n < 2 then return n end
    return fib(n - 1) + fib(n - 2)
  end
  return fib(x)
end
```

This phase replaces the panic with a proper lowering. **No
upvalue capture** — the nested body still cannot reach into the
outer function's locals; that's the closure phase (Phase 2.5c).
What lands here is the static-name machinery: nested function
definitions are hoisted into `chunk.functions` like top-level
ones, with sibling forward-reference and self-recursion working
through the existing `function_names` registry.

## Decision

### 1. Two pass-1 / pass-2 splits, one shared pair of helpers

`lower()` already does a pass-1 / pass-2 split for the chunk's
top-level FunctionDefs. The new nested case mirrors it — but
inside `LowerCtx::lower_function_body` rather than the standalone
`lower()` function.

To avoid duplicating the registration + body-lowering logic, two
free helpers are extracted:

```rust
fn register_function_signature(
    name: &str,
    params: &[String],
    function_names: &mut HashMap<String, FuncId>,
    functions: &mut Vec<HirFunction>,
) -> FuncId;

fn lower_into_function(
    fid: FuncId,
    params: &[String],
    body: &[Stmt],
    function_names: &HashMap<String, FuncId>,
    functions: &mut Vec<HirFunction>,
) -> Result<(), HirError>;
```

Both `lower()` and the new nested-FunctionDef arm in
`lower_stmt` route through the same code paths. Pure relative to
their mutable arguments — no implicit state.

### 2. Pass-1 inside `lower_function_body`

Before lowering body statements, the function body's top-level
`FunctionDef` stmts are pre-scanned and their signatures
registered in `self.function_names` + placeholder `HirFunction`s
pushed into `self.functions`:

```rust
for s in stmts {
    if let StmtKind::FunctionDef { name, params, .. } = &s.kind {
        register_function_signature(
            name, params,
            &mut self.function_names,
            &mut self.functions,
        );
    }
}
```

This is what makes sibling forward references work:

```lua
local function outer()
  local function g(n) return h(n) + 1 end   -- references h
  local function h(n) return n * 10 end     -- declared later
  return g(5)
end
```

`g`'s body lowers after `h` is already in `function_names`, so
the call resolves cleanly.

### 3. Pass-2 in `lower_stmt::FunctionDef`

When the lower-stmts phase encounters the FunctionDef, the body
is filled in via `lower_into_function`, which:

- Snapshots `pre_count = functions.len()`.
- Reads `external_kinds` from the placeholder's `params`
  (default Number; Phase 2.5e's call-site refinement applies at
  top-level only).
- Spins up a fresh `LowerCtx::for_function`, lowers the body,
  copies `locals` / `body` / `ret_kinds` into
  `functions[fid.0]`.
- Hoists any anonymous functions registered in the inner
  `LowerCtx` (indices `≥ pre_count`) up into the outer table —
  same mechanism `lower()` already used for the top-level case.

The `FunctionDef` statement itself produces no runtime ops in
the enclosing body, since codegen reads function definitions
from `chunk.functions`. The lowered statement is therefore a
single empty `HirStmtKind::Block { stmts: vec![] }`.

### 4. Upvalue capture remains a hard error

A nested function body cannot reference the outer function's
parameters or locals. Attempting to do so surfaces
`HirError::UndefinedName`:

```lua
local function outer(x)
  local function inner()
    return x      -- UndefinedName "x"
  end
  return inner()
end
```

This is the deliberate boundary for Phase 2.5f / Phase 2.5c
(closures). Lifting it requires either lambda-lifting at HIR
time, a heap-allocated environment, or a runtime
(fn_ptr, env_ptr) pair — none of which are in scope here. The
Phase 2.5c ADR will document the choice.

### 5. Calling-side resolution is unchanged

`lower_call`'s priority order — Function-kind local with a
known FuncId, then Function-kind local with no FuncId
(parameter), then `function_names`, then `Builtin::from_name` —
already handles the nested case. The nested function lands in
`function_names`, so resolution falls through to the third arm
naturally.

### CA invariants

| Layer    | Change                                                       |
|----------|--------------------------------------------------------------|
| Lexer    | None                                                         |
| Parser   | None                                                         |
| AST      | None                                                         |
| HIR      | Pass-1 in `lower_function_body`; non-`unimplemented` arm in `lower_stmt`; two extracted helpers |
| Codegen  | None                                                         |

The codegen layer is undisturbed — every nested `HirFunction` is
just another entry in `chunk.functions` and emits via the
existing `emit_function` pipeline.

## TDD Process

1. **Tidy First (review only).** No behaviour-preserving
   refactor was warranted before the feature work — the existing
   top-level pass-1 / pass-2 lowering was clean. The shared
   helper extraction landed during Step 4 (Refactor) once the
   third call site emerged, matching the "rule of three" trigger.
2. **Red.** Three HIR unit tests + six integration tests
   referenced the not-yet-existent nested-FunctionDef path.
   Three of them panicked on `unimplemented!`.
3. **Green.** Pass-1 added to `lower_function_body`; the
   `unimplemented!` arm replaced with proper lowering. All tests
   passed at 505.
4. **Refactor.** With three call sites of the
   "register a FunctionDef" / "lower its body" pattern in play
   (top-level pass-1, top-level pass-2, nested), the duplicated
   logic was lifted into `register_function_signature` and
   `lower_into_function`. The 505 test count was unchanged
   throughout, confirming the refactor preserved behaviour.

## Alternatives Considered

- **Desugar `local function f` to `local f; f = function() ...
  end`** at the AST level. Rejected because Lua specifies the
  name as visible inside its own body for recursion, but the
  desugared form would not (an `f` declared on a left-hand-side
  is not in scope inside its right-hand-side initialiser). A
  faithful desugar would have to manually pre-declare `f` and
  patch the LocalInit in two phases — exactly the work we're
  doing in the HIR pass-1, just in a more roundabout shape.
- **Recurse on `lower_into_function` from inside `lower_stmt`
  without pre-registration.** Would compile a single nested
  function correctly but break sibling forward-reference, which
  the test suite explicitly covers.
- **Track upvalues now and partially capture.** Phase 2.5c's
  scope. Mixing a half-implementation here would entangle the
  no-capture story with the capture story; cleaner to keep them
  separate.

## Consequences

- HIR adds the two free helpers
  (`register_function_signature`, `lower_into_function`) used by
  three call sites.
- Pass-1 in `lower_function_body` mirrors the top-level pass-1
  in `lower()`.
- Three HIR unit tests + six integration tests cover the basic
  nested call, recursive self-reference (factorial,
  Fibonacci), two sibling functions called in sequence,
  forward-reference between siblings, the upvalue rejection,
  and a three-level-deep nesting.

## Out of Scope

- **Upvalue capture / closures** — Phase 2.5c.
- **Mutual recursion across siblings** when the **declaration**
  order matters. Lua's actual semantics treat `local function f`
  as `local f; f = function ... end`, which means a sibling `g`
  defined **after** `f` and called from `f`'s body works only
  because Lua resolves names at call time, not at definition
  time. We achieve the same observable effect via the pass-1
  pre-registration (sibling forward-reference works), but
  without the runtime late-binding semantics — a static
  divergence that's fine until first-class re-binding lands.
- **Nested FunctionDef inside `if` / `while` / `for` / `do`
  blocks.** The pass-1 walker only inspects the body's top
  level. A nested-in-block FunctionDef would still hit the
  `lower_stmt` arm but wouldn't have a pre-registered
  placeholder, panicking at the `expect`. This is a deliberate
  limitation; lifting it requires walking nested blocks during
  pass-1, which we'll do when a test demands it.
