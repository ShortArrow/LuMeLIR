# 0018. Phase 2.5b.2: Functions as Arguments via `func.call_indirect`

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** ShortArrow

## Context

Phase 2.5b shipped first-class function values whose `FuncId` is
known statically — `local f = function() end` and the alias chain
`local g = f`. Phase 2.5b.2 extends that to **passing functions as
arguments**, the typical Lua higher-order pattern:

```lua
local function apply(g, x)
  return g(x)
end

local double = function(x) return x * 2 end
print(apply(double, 7))            -- 14
```

Inside `apply`, the parameter `g` is *not* statically a particular
function — it could be any function value the caller chose. That
means a runtime function value, dispatched via `func.call_indirect`.

## Decision

### 1. `ValueKind::Function` carries arity, not `FuncId`

```rust
pub enum ValueKind {
    Number, Bool, Nil,
    Function(usize),     // arity (number of f64 params)
}
```

`FuncId` was only useful for static dispatch; for a parameter we
don't have it. Arity (currently always with `Number` ret) is the
information call sites actually need.

### 2. `LocalInfo` gains `func_id: Option<FuncId>`

```rust
pub struct LocalInfo {
    pub name: String,
    pub kind: ValueKind,
    pub func_id: Option<FuncId>,    // Some when the local was bound
                                    // to a known function (Phase 2.5b
                                    // static-dispatch path).
}
```

`Some(fid)` is set on `local f = function() ... end` and on aliases
of such locals. Function parameters of `Function`-kind have
`func_id = None`.

### 3. `Callee` gains `Indirect(LocalId)`

```rust
pub enum Callee {
    Builtin(Builtin),
    User(FuncId),         // existing static dispatch
    Indirect(LocalId),    // dynamic dispatch via function-kind local
}
```

`lower_call`'s priority:

1. Identifier resolves to a Function-kind local with `func_id =
   Some(fid)` → `Callee::User(fid)` (preserves the Phase 2.5b path).
2. Identifier resolves to a Function-kind local with `func_id =
   None` (i.e. a parameter) → `Callee::Indirect(local_id)`.
3. Identifier matches a name in `function_names` → `Callee::User(fid)`.
4. Identifier matches a builtin → `Callee::Builtin(b)`.
5. Otherwise `UnknownFunction`.

Arity is checked in cases 1, 2, and 3 against `args.len()`.

### 4. Param kind back-inference

The AST has no type annotations, so we can't tell from `function(g, x)`
alone that `g` will be called as a function. Phase 2.5b.2 introduces
a **pre-scan** of the body before lowering each function:

```rust
fn infer_param_kinds(body: &[Stmt], params: &[String]) -> Vec<ValueKind>
```

The scan walks the AST recursively, rejecting parameters as
`Function(arity)` when it sees `g(args)` for a parameter name `g`,
and leaves the rest as `Number` (the existing default). Arity is
the static arg count at the call site. Conflicting arities for the
same parameter are caught at `lower_call`-time as an
`ArityMismatch`.

This is run inside `LowerCtx::for_function` before the params are
declared, so each param gets the correct kind before the body is
lowered.

### 5. Codegen storage: SSA values, not slots

For Phase 2.5a/b we treated every local as a stack slot (alloca +
load/store). For `ValueKind::Function`, MLIR's `!func.func<...>`
type doesn't fit cleanly into `llvm.alloca` (the LLVM dialect's
alloca expects LLVM-typed elements, and the func dialect's
function-typed value is different).

We sidestep the issue by **not allocating slots for Function-kind
locals**. Instead the codegen carries a side map:

```rust
function_values: HashMap<LocalId, Value<'c, 'a>>
```

Inserted on `LocalInit` (the value comes from `func.constant
@<mangled>` or from another Function-kind local via copy), looked
up on call sites and on parameter passing. Function parameters
populate the map from the function's block arguments.

Phase 2.5b.2 doesn't allow reassignment of a Function-kind local
(that already returned `TypeMismatch` in 2.5b), so the SSA
value model is correct.

### 6. Function call sites

- `Callee::User(fid)`: existing `func.call @<mangled>` path.
- `Callee::Indirect(local_id)`:
  1. `let callee_val = function_values[local_id]` — already an
     `!func.func<...>`-typed SSA value.
  2. Emit `func.call_indirect %callee_val(%args) : (...) -> (f64)`.

### 7. Function-typed function parameters

`emit_function`'s parameter type list now respects each
`hir_fn.params[i].kind`:

- `Number` → `f64`.
- `Function(arity)` → `FunctionType::new(&[f64; arity], &[f64])`.

Block arguments are wired to `function_values` for Function params
or to alloca slots for Number params, exactly as the kind dictates.

## Alternatives Considered

- **Arity-uniform calling convention** (everyone is `(f64) -> f64`).
  Simple but kills 2-arg first-class functions. Rejected.
- **Annotate parameter types in source** (`function(g: function, x)`).
  Diverges from Lua. Rejected.
- **Runtime tag for function-kind values**. Needs a tagged-union
  layer; defeats the static type model. Rejected.
- **Slots with `!llvm.ptr` storage and bitcasts**. Would technically
  work but introduces verifier-fragile conversions. The SSA-only
  approach for Function-kind locals is cleaner and only viable
  because Phase 2.5b.2 forbids Function-kind reassignment.

## Consequences

- `ValueKind::Function` payload changes from `FuncId` to `usize`
  (arity). Every destructure site updates.
- `LocalInfo` gains `func_id`. Every constructor site updates.
- `Callee` gains `Indirect(LocalId)`. Codegen and HIR-side
  exhaustive matches update.
- Function-kind locals no longer have alloca slots. Codegen carries
  a `function_values` map alongside `slots`.
- `infer_param_kinds` is the new piece in HIR; AST is unchanged.

## Out of Scope

- Functions as **return** values → Phase 2.5b.3 (ret_kind extension).
- Reassigning a function-kind local (`f = g`) → Phase 2.5c (needs
  storage of function values, not just SSA flow).
- Arity-polymorphic function values (`(f64) -> f64` and `(f64,f64)
  -> f64` in the same slot) → needs runtime tag.
- Closures (`function() return outer end`) → Phase 2.5c.
- Multiple return values, varargs → Phase 2.5d / later.
- Non-Number param/return kinds → Phase 2.5e.
