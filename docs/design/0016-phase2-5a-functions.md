# 0016. Phase 2.5a: Top-Level `local function` + `return` + Recursion

- **Status:** Accepted
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.4 closed out the loop control-flow set. The next milestone is
**user-defined functions**. Lua's full function model is large (first-
class values, closures, multiple returns, varargs, methods), so we
split Phase 2.5 into four sub-phases. This ADR covers the smallest
viable starting point — **2.5a**:

```lua
local function add(a, b)
  return a + b
end

local function fact(n)
  if n == 0 then return 1 end
  return n * fact(n - 1)
end

print(add(2, 3))                  -- 5
print(fact(5))                    -- 120
```

The Phase 2.5a contract:
- `local function NAME(PARAMS) BODY end` only — no anonymous form.
- All parameters and return values are **`Number` (f64)**. Bool/Nil
  param/ret kinds arrive in Phase 2.5c.
- **Single** return value (multiple-return is Phase 2.5b).
- **Recursion** is supported.
- **No closures**: the function body cannot reference outer locals.
- Function values are **not first-class** (no `local f = some_fn`).

## Decision

### 1. AST and lexer

Two new keywords: `function`, `return`.

```rust
StmtKind::FunctionDef { name: String, params: Vec<String>, body: Chunk }
StmtKind::Return      { value: Option<Expr> }
```

`local function f(...) ... end` is the only entry point — `parse_local`
peeks one ahead for `function` and dispatches to `parse_function_def`.

### 2. HIR

```rust
pub struct FuncId(pub usize);

pub struct HirFunction {
    pub name: String,                 // user-visible
    pub mangled_name: String,         // MLIR symbol (`user_f_<idx>`)
    pub params: Vec<LocalInfo>,       // each param's slot meta
    pub locals: Vec<LocalInfo>,       // params + body locals
    pub body: Vec<HirStmt>,
    pub ret_kind: Option<ValueKind>,  // None == void
}

pub struct HirChunk {
    pub locals: Vec<LocalInfo>,
    pub stmts: Vec<HirStmt>,
    pub functions: Vec<HirFunction>,  // NEW
}

HirStmtKind::Return { value: Option<HirExpr> }
HirExprKind::Call    { callee: Callee, args: Vec<HirExpr> }
pub enum Callee { Builtin(Builtin), User(FuncId) }
```

`infer_kind` gains a third parameter `&[HirFunction]` so the call's
result kind can be looked up. All callers update.

### 3. Lowering

A two-pass walk over the top-level chunk:

1. **Function-table pass**: collect every `local function f` and
   register `f` in `LowerCtx::function_names: HashMap<String, FuncId>`.
   This makes `f` reachable from inside its own body, supporting
   recursion.
2. **Body pass**: lower each function's body in a *fresh* lowering
   context (separate `LowerCtx` instance) so loop break stacks,
   read-only sets, and scope chains do not leak across function
   boundaries.

`StmtKind::Return` lowers via the same desugar pattern as Phase 2.4's
`break`:

- Each function body opens with two synthetic locals at top:
  `_returned: Bool` (init `false`), `_ret_value: Number` (init `0`).
- `return e`  →  `_ret_value = e; _returned = true`.
- `return`    →  `_returned = true`.
- Each body statement is wrapped in `if not load(_returned) then ... end`
  so post-`return` code is skipped.
- `LowerCtx::in_function: Option<(LocalId /* _returned */, LocalId /* _ret_value */)>`
  drives both the lowering of `Return` and the rejection of `return` at
  top-level (`HirError::ReturnOutsideFunction`).

### 4. Codegen

Each `HirFunction` becomes a `func.func @user_<name>_<idx>` MLIR
symbol, emitted before `@main`. The body uses the same alloca-hoist
pattern as `emit_main`:

- Param block-arguments enter the entry block, get stored into their
  alloca slots.
- `_returned`, `_ret_value`, and any user locals are alloca'd at entry.
- Body statements emit through the existing `emit_stmts` path.
- The function ends with a single `func.return` — `load(_ret_value)`
  for value-returning functions, no operand for `void`.

User calls go through `func.call @user_<name>_<idx>(args...) -> (ret)`.
Builtin `print` continues through its existing path.

### 5. Symbol naming

`user_<name>_<idx>` where `<idx>` is the function's position in
`chunk.functions`. Disambiguates from `@main` and from any future
shadowing of function names.

## Alternatives Considered

- **First-class function values from the start**. Adding
  `ValueKind::Function(...)` plus closure semantics is much larger
  than 2.5a. We commit to it in Phase 2.5b after the basics work.
- **`func.return` mid-body**. `func.return` is a region terminator;
  emitting it in the middle of a `scf` region requires unstructured
  CFG, which fights with our `scf.while`/`scf.if` lowering. The
  flag-based desugar reuses the proven Phase 2.4 pattern.
- **Force every function to be value-returning** (synthesise `return 0`
  for void). Forces a Lua-incorrect signature. Rejected.
- **`llvm.func` instead of `func.func`**. We stay with `func` because
  the `arith` ops we already emit are best paired with `func.func`'s
  region semantics.

## Consequences

- `Keyword` +2 (`Function`, `Return`).
- `StmtKind` +2; `HirStmtKind` +1.
- `HirError` +2 (`ReturnOutsideFunction`, `UnknownFunction`).
- `HirChunk` gains `functions`. Every place that constructs a
  `HirChunk` literally is updated (the lowering site and tests).
- `HirExprKind::Call` field shape changes (similar blast radius to
  Phase 2.3c's `BinOp::And/Or`).
- `infer_kind` signature gains `&[HirFunction]`; all callers update.
- `LowerCtx` gains `functions`, `function_names`, `in_function`.
- `emit_function` is the new codegen entry point; `emit_module`
  iterates `chunk.functions` first, then emits `@main`.
- The IR produced for a body with no `return` still includes the
  `_returned` / `_ret_value` slots — LLVM optimisation folds them
  for the common case.

## Out of Scope (deferred)

- Closures / upvalues → Phase 2.5b.
- Multiple return values, `select` → Phase 2.5b.
- Anonymous expression-form `function() ... end` → Phase 2.5b.
- `local f = some_function` (first-class function values) → 2.5b.
- Bool/Nil parameter or return kinds → Phase 2.5c.
- Variadics `...` → Phase 2.5d or later.
- Global `function f() end` → Phase 2.6+ (needs tables).
- Methods `obj:method()` → Phase 2.6+ (needs tables).
