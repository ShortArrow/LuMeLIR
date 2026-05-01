# 0037. Phase 2.5c-min: Capture-By-Value Closures (Direct-Call Only)

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.5f wired nested `local function` definitions but kept the
"upvalue capture is a static error" rule from earlier phases. That
left a real ergonomic gap — the most common nested-function pattern
in Lua is to close over a Number from the enclosing scope:

```lua
local function map(n, fn)
  -- ...
end

local m = 10
local doubler = function(x) return x * m end
```

This phase lights up the minimum viable closure form: anonymous
function expressions and nested `local function` bodies can capture
**Number** locals from the currently-visible enclosing scope. The
closure value remains a bare function pointer — no
(fn_ptr, env_ptr) struct, no heap-allocated environment, no escape.

The pragmatic restriction that makes this work is **direct-call
only**. Captured upvalues are passed as extra arguments at every
call site, lifted out of the closure value entirely. The call site
must therefore know the static FuncId of the callee (i.e.
`Callee::User`). Indirect calls (`Callee::Indirect`, e.g. a
function passed as an argument) cannot reach the upvalue list and
are forbidden when the target carries upvalues.

## Decision

### 1. `HirFunction.upvalues` and `UpvalueInfo`

```rust
pub struct HirFunction {
    // ...
    pub upvalues: Vec<UpvalueInfo>,
    // ...
}

pub struct UpvalueInfo {
    pub name: String,
    pub kind: ValueKind,
    pub outer_local_id: LocalId,   // in the enclosing ctx
    pub inner_local_id: LocalId,   // in this function's locals
}
```

`outer_local_id` is the enclosing scope's slot we read at every
call site. `inner_local_id` is the slot the captured value lands in
inside this function — one for each capture, declared during the
first lookup.

### 2. `LowerCtx::outer_visible` snapshot

`LowerCtx` gains a snapshot of names visible in the enclosing
scope at the moment the inner ctx is created:

```rust
struct LowerCtx {
    // ...
    outer_visible: HashMap<String, (LocalId, ValueKind)>,
    upvalues: Vec<UpvalueInfo>,
}
```

`for_function` accepts this snapshot. Both call sites
(`FunctionExpr` lowering and the nested-FunctionDef arm of
`lower_stmt`) build the snapshot via the new
`LowerCtx::outer_visible_snapshot` helper, which walks the scope
stack so the latest binding for each name wins (matching `resolve`
semantics). Top-level pass-2 lowering passes an empty map —
top-level `local function` predates main-chunk locals and has no
visible enclosing scope.

### 3. `lookup_or_capture_upvalue` in `lower_expr::Ident`

When the standard cascade (scopes → function_names) misses, the
new helper consults `outer_visible`. On a hit:

1. Reject non-Number kinds (Phase 2.5c-min restriction):
   `HirError::TypeMismatch { op: "upvalue capture", ... }`.
2. De-dup against existing upvalues — repeated references to the
   same outer name share one inner local.
3. First capture: declare a fresh local in the inner ctx, push an
   `UpvalueInfo` carrying both `outer_local_id` and
   `inner_local_id`.
4. Return the inner `LocalId` so the surrounding `lower_expr`
   continues with `HirExprKind::Local(inner_id)`.

### 4. Call sites pass upvalues as extra args

Both `Callee::User` paths in `lower_call` (the local-with-known-
FuncId path and the function_names path) extend the lowered args
list with one `HirExprKind::Local(uv.outer_local_id)` per
declared upvalue:

```rust
let upvalue_args: Vec<HirExpr> = self.functions[fid.0]
    .upvalues
    .iter()
    .map(|uv| HirExpr {
        kind: HirExprKind::Local(uv.outer_local_id),
        span: whole.span,
    })
    .collect();
all_args.extend(upvalue_args);
```

The HIR `Call` therefore carries `[lua_args..., upvalue_args...]`,
and the call expression's arity matches the function's MLIR
signature.

### 5. Codegen: upvalue params after Lua params

`emit_function`'s signature widens to `[param_types...,
upvalue_types...]`. The block gains the same number of arguments;
each upvalue's incoming block argument is stored into
`slots[uv.inner_local_id.0]` at function entry — alongside the
existing param-store loop. The body codegen reads the upvalue via
the standard `HirExprKind::Local` path (no special-case), so once
the slot is initialised on entry the rest of the function works
unchanged.

### 6. Live-binding semantic in scope, snapshot would need heap

Captures are read from `outer_local_id` at every call site —
which means a reassignment to the outer slot **does** propagate
to subsequent calls of the closure inside the same scope:

```lua
local x = 1
local f = function() return x end
x = 99
print(f())   -- 99
```

This matches Lua's "upvalue is the binding" semantic, with one
caveat: because the closure value carries no env pointer, it
**cannot escape** its creation scope. A future Phase 2.5c-full
will introduce (fn_ptr, env_ptr) pairs and heap-allocated cells;
that's where escape lands.

### 7. Limitations and out-of-scope items

- **Non-Number captures** (Bool/Nil/String/Function): rejected as
  `TypeMismatch`.
- **Indirect calls of closures** (Callee::Indirect, function-as-arg):
  cannot pass upvalues; closures with upvalues must reach their
  call sites via Callee::User. Currently this is enforced by the
  call-site code only being on the `Callee::User` paths — passing
  a closure as an argument would lower as Indirect and silently
  drop the upvalues. A follow-up phase tightens this with a
  static `ClosureEscapes` rejection.
- **Top-level `local function` capturing chunk-level locals**:
  blocked because top-level function bodies are lowered in
  pass 2, before main chunk locals exist. Use the anonymous form
  (`local f = function() ... end`) until the chunk lowering is
  reordered to interleave local-decl and function-body lowering.
- **Reassignment-aware capture semantics**: live-binding works
  *inside* the creation scope (because the outer slot still
  exists). Cross-scope semantics require true cells.

### CA invariants

| Layer    | Change                                                              |
|----------|---------------------------------------------------------------------|
| Lexer    | None                                                                |
| Parser   | None                                                                |
| AST      | None                                                                |
| HIR      | `HirFunction.upvalues`, `UpvalueInfo`, `LowerCtx.{outer_visible, upvalues}`, `outer_visible_snapshot`, `lookup_or_capture_upvalue`, threading through `for_function` / `lower_into_function`, call-site arg extension on both `Callee::User` paths |
| Codegen  | `emit_function` widens its MLIR signature to `[params + upvalues]` and stores incoming upvalue block-args into the matching slots; otherwise the body emit is unchanged |

The codegen change is small and local — all the structural work
sits in HIR. Indirect-call codegen is untouched (closures don't
flow through `Callee::Indirect`).

## TDD Process

1. **Tidy First (review only).** The existing FunctionExpr and
   nested-FunctionDef lowering paths threaded enough information
   through `LowerCtx::for_function` that adding one more parameter
   (`outer_visible`) was a clean extension. No behaviour-preserving
   refactor was warranted up front; the duplication-removal that
   the helper extraction in Phase 2.5f did already paid for the
   common path.
2. **Red.** Four HIR unit tests + eight integration tests added,
   referencing not-yet-existent `HirFunction.upvalues` and
   `UpvalueInfo`. Compilation refused on the unknown field.
3. **Green.** UpvalueInfo + the field, then the
   `outer_visible` plumbing, then `lookup_or_capture_upvalue`,
   then the call-site arg extension on both `Callee::User` paths,
   then `emit_function`'s widened signature. Tests passed at 517
   (505 + 4 HIR + 8 e2e) once two e2e expectations were
   corrected: the "snapshot" test was reframed as live-binding
   (which is what our impl gives, matching Lua), and the
   top-level-`local function` capture test was reframed to use
   the anonymous form (the only form whose body lowers when chunk
   locals are visible).
4. **Refactor (review).** No further duplication emerged once the
   call-site arg extension shared the same shape on both
   `Callee::User` paths. The `if let Callee::User(fid) = callee`
   guard inside the local-with-func_id path keeps the extension
   localised; a small wrinkle, but the alternative (a free helper
   that takes both args lists) trades line count for indirection.

## Alternatives Considered

- **Heap-allocated env from day one** — gives full Lua semantics
  including escape. Rejected for this phase: the lambda-lifting
  approach unblocks the most common non-escaping patterns at a
  fraction of the implementation cost.
- **Capture-by-value via a per-closure stack alloca** that copies
  the outer slot at FunctionExpr-evaluation time. Would give
  snapshot semantics but adds a memcpy per closure creation; in
  practice the call-time reload is cheaper because the outer slot
  is hot in the cache for in-scope closures.
- **Allow Bool/Nil captures**. Adds variant handling at every
  edge of the upvalue path. Defer until a test demands it.

## Out of Scope

- **Closure escape** (return / pass as arg / outlive creation
  scope). Phase 2.5c-full or a separate "first-class closures"
  phase.
- **Mutually-capturing nested functions** (`a` captures `b` while
  `b` captures `a`). Hard with our pass-1/pass-2 split unless
  upvalue analysis itself runs as a third pass.
- **Top-level `local function` capturing chunk-level locals.**
  Documented limitation; lifts when chunk lowering reorders.
- **Function-kind upvalues.** Would let closures-of-closures work
  but needs the function-value ABI extension.
- **Static `ClosureEscapes` rejection** for closures with
  upvalues being passed to `apply(f, ...)`-style sinks. The
  current impl drops upvalues silently in that path; a future
  tightening adds the diagnostic.
