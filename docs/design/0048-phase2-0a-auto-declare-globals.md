# 0048. Phase 2.0a: Auto-Declare Globals at Chunk Top Level

- **Status:** Accepted
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

Until now, every name had to be introduced with `local`:

```lua
local PI = 3
local function area(r) return PI * r * r end
print(area(5))
```

Real-world Lua scripts rely heavily on the unmarked top-level
form — `PI = 3` is a global in standard Lua, accessible from
anywhere via `_ENV`. Without any kind of bare-name binding,
users hit the wall on day-1 with their existing code.

Full Lua globals require a tagged-value table (`_ENV`),
metatables, and dynamic typing. That's a Phase-2.6+ structural
project. But a useful **subset** falls out cheaply: treat a
top-level bare assignment as syntactic sugar for `local`.

## Decision

### Auto-declare on bare assign at chunk top level

In `lower_stmt`'s `StmtKind::Assign` arm, when the LHS name
doesn't resolve in any scope:

```rust
if self.in_function.is_some() {
    return Err(HirError::UndefinedName { ... });
}
if self.function_names.contains_key(name) {
    return Err(HirError::TypeMismatch { ... });
}
let value = self.lower_expr(value)?;
let kind = infer_kind(&value, ...);
let id = LocalId(self.locals.len());
self.locals.push(LocalInfo { name: name.clone(), kind, func_id });
self.scopes[0].insert(name.clone(), id);  // chunk scope, not innermost
return Ok(HirStmt {
    kind: HirStmtKind::LocalInit { id, value },
    span: stmt.span,
});
```

Three rules apply:

1. **Top-level only.** When `in_function` is `Some`, the
   unresolved-name path still errors. Lua treats globals
   identically inside and outside functions; we don't yet,
   because supporting that would require either (a) implicit
   chunk-local capture in every function or (b) a real
   `_ENV` table — both are bigger phases.
2. **Insert at chunk scope (`scopes[0]`)**, not the
   innermost frame. A `do … end` block at top level may
   auto-declare; the binding survives the block exit.
3. **No shadowing of `function_names`.** A bare `helper = 99`
   when `local function helper` exists rejects with
   `TypeMismatch`. The two namespaces don't merge.

### Kind is inferred and stable

Auto-declared globals follow the same single-kind rule as
locals: kind is fixed at first assignment, subsequent
assignments must match. Differs from Lua, which is
dynamically typed, but matches every other slot in our
compiler.

### Capture for free via 2.5c.1

Once the name is a chunk-level local, top-level
`local function` bodies can capture it as an upvalue via
the path established in ADR 0042. So:

```lua
PI = 3
local function area(r) return PI * r * r end
print(area(5))   -- 75
```

works without any further codegen change. Number / Bool /
Nil / String captures all carry over from ADR 0043.

### CA invariants preserved

| Layer    | Change                                          |
|----------|-------------------------------------------------|
| Lexer    | None                                            |
| Parser   | None — `Assign` was already a statement form    |
| AST      | None                                            |
| HIR      | One arm in `lower_stmt::Assign` extended with the auto-declare path |
| Codegen  | None — the emitted `LocalInit` is the same shape as `local x = …` |

## TDD Process

1. **Red.** 10 e2e tests covering basic Number / String / Bool
   global, kind-stable reassignment, kind-change rejection,
   capture-by-top-level-fn, function-body rejection,
   `do … end` chunk-scope, `function_names` shadow rejection,
   and a `local x = 1` regression. Six failed
   (the new auto-declare path didn't exist); four passed
   (boundary cases that still errored via the old paths).
2. **Green.** Added the auto-declare branch in the `Assign`
   arm. The first attempt put the binding in the innermost
   scope, which broke the `do … end` test — fixed by
   inserting into `scopes[0]`.
3. **Refactor.** Updated one HIR unit test
   (`lower_assign_to_undefined_name_errors`) to test the new
   inside-function-still-errors boundary, and added a sibling
   unit test pinning the new auto-declare behaviour at chunk
   level. No further duplication emerged.

## Alternatives Considered

- **Treat bare assign anywhere as auto-declare.** Inside a
  function, `x = 1` would create a function-local. This
  diverges from Lua more sharply (Lua makes it global) and
  also introduces a real footgun: typos like `pirnt = 1`
  wouldn't error. Defer until full `_ENV` lands.
- **Keep bare assign as an error and require `local`
  everywhere.** The current behaviour, but it forces every
  copied-from-the-internet Lua snippet to be rewritten.
  Rejected.
- **Distinguish "globals" from "locals" with a separate
  symbol table** so cross-function reads work. Doable but
  adds plumbing for a use case (read globals from inside a
  function without explicit capture) that 2.5c.1 mostly
  covers via the upvalue path. The capture path even
  matches Lua's behaviour more closely (every read snapshots
  the slot at call time). Defer.
- **Reject when the global's first assignment isn't at
  the chunk's outermost statement** (i.e. require globals to
  appear before any block). Easier to reason about but
  artificial; Lua puts no such restriction.

## Consequences

- HIR adds ~25 lines (one new arm in `Assign`).
- 10 new e2e tests + 1 new HIR unit test + 1 reframed unit
  test. Total green at 633.
- The unblocked pattern is idiomatic top-level Lua: constants,
  configuration values, and the `function fn_name() ... end`
  global-function form (already supported via
  `local function`).
- Lua-divergence: globals don't propagate into nested
  function bodies without explicit capture. Documented
  honestly in this ADR.

## Out of Scope

- **Cross-function global access without capture** — pending
  full `_ENV` semantics or a Phase 2.0b lift that auto-
  captures every chunk-level local for every function body.
- **Dynamically-typed globals** — pending tagged values.
- **Globals inside function bodies** (`function f() x = 1
  end` creating a global). Also pending `_ENV`.
- **`local`-vs-global declaration disambiguation**. Inside
  functions, plain `x = 1` still errors; users must use
  `local x = 1`. The error message is clear enough thanks
  to ADR 0045's diag layer.
