# 0020. Phase 2.5e: Bool/Nil Parameters and Return Values

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Phase 2.5a fixed user-defined function signatures to "Number params,
Number-or-void return". Phases 2.5b/2.5b.2/2.5b.3 widened the **type**
of values that can flow through function values (functions
themselves) but kept the underlying scalar type-set unchanged. The
practical consequence: predicate functions

```lua
local function pos(x) return x > 0 end
local function negate(b) return not b end
```

both fail to lower — `pos` because `return BoolExpr` was a
`TypeMismatch`, `negate` because the param `b` defaulted to Number
and `not b` returns Bool but the body never sees that as a refinement
of the Number assumption (and `negate(true)` rejects the arg).

Phase 2.5e admits Bool and Nil into the user-function signature
without touching first-class function values. Predicates and
`nil`-returning helpers are common Lua patterns; the design is the
smallest extension of the existing static type system that makes them
work.

## Decision

### 1. Returns: every kind, cross-checked across paths

`lower_stmt::Return` no longer enforces Number-or-Function;
**every** `ValueKind` is admissible. The slot's kind is upgraded on
first value-returning return, and every subsequent return in the
same body must agree:

```rust
if let Some(prev) = self.in_function_ret_kind && prev != v_kind {
    return Err(HirError::TypeMismatch { ... });
}
self.in_function_ret_kind = Some(v_kind);
self.locals[ret_value_id.0].kind = v_kind;
```

### 2. Parameters: chunk-level call-site inference

The AST has no type annotations, so a function definition alone
cannot tell us a param's kind. Phase 2.5e adds a chunk-level pre-scan
that walks every call site of every top-level `FunctionDef` and
records the static literal kind of each argument:

```rust
fn ast_arg_kind(expr: &Expr) -> ValueKind {
    match &expr.kind {
        ExprKind::Bool(_)   => ValueKind::Bool,
        ExprKind::Nil       => ValueKind::Nil,
        ExprKind::Number(_) => ValueKind::Number,
        ExprKind::UnaryOp { op: Neg, operand: Number(_) } => ValueKind::Number,
        _ => ValueKind::Number,    // fall back to the historical default
    }
}
```

The first call site for a function wins; later call sites with
different kinds fail in `lower_call`'s existing arg-vs-param kind
check (`TypeMismatch`). This is run once in `lower()` between pass 1
(register signatures) and pass 2 (lower bodies), and the resulting
kinds are fed into `LowerCtx::for_function` as `external_kinds`.

### 3. Body-pre-scan vs. call-site inference: Function wins

The existing 2.5b.2 body-pre-scan (`infer_param_kinds`) marks a
parameter `Function(arity)` when it appears as a callee `g(args)`.
That is decisive: the body itself proves the param is a function. If
both inferences fire, the body wins:

```rust
let kind = match body_kinds[i] {
    ValueKind::Function(_) => body_kinds[i],   // body proves it
    _ => external_kinds[i],                     // otherwise call-site
};
```

### 4. `_ret_value` slot init by kind

Phase 2.5b.3 began conditionally suppressing the synthetic
`_ret_value = 0.0` init for Function-kind ret slots. Phase 2.5e
generalises: the prelude emits a kind-appropriate default (`0.0` for
Number, `false` for Bool, `nil` for Nil, none for Function) so an
early-exit path still loads a sane value where one exists.

### 5. Codegen: Bool/Nil ret types

`ret_mlir_types` extends to map `Bool` and `Nil` to `i1`. Both
function declaration and call-site result types use the helper, so
they remain in lock-step. The trailing `func.return` for Bool/Nil
ret_kind loads the slot as `i1`:

```rust
Some(ValueKind::Bool) | Some(ValueKind::Nil) => {
    let ret_value_idx = hir_fn.params.len() + 1;
    vec![emit_load(&block, slots[ret_value_idx], types.i1, loc)]
}
```

`param_mlir_type` already mapped Bool/Nil to `i1`, so parameter
declaration needed no change.

### 6. Argument-kind compatibility relaxed

`lower_call`'s arg-vs-param compatibility table grows two entries:

```rust
(ValueKind::Bool, ValueKind::Bool) => true,
(ValueKind::Nil,  ValueKind::Nil)  => true,
```

Function and Number cases are unchanged.

## Alternatives Considered

- **Add explicit type annotations to the parser** (`function pos(x:
  number) -> bool`). Diverges from Lua. Rejected.
- **Refine param kinds during body lowering, then re-lower bodies on
  conflict** — would catch deeper inference (e.g. `param + 1` proves
  Number). Larger machinery and re-lowering is brittle. Rejected for
  now; can revisit if practical patterns demand it.
- **Treat all params as a heterogeneous "Any" until a use site
  refines them**. Conflicts with the static type system; demands a
  tagged-value layer. Rejected.
- **Limit Phase 2.5e to Bool/Nil returns only** — strictly smaller
  but cuts off the negate-style pattern. Rejected; the call-site
  inference is mechanical enough to ship together.

## Consequences

- HIR `lower_stmt::Return` accepts every value kind; cross-check
  applies to all kinds uniformly.
- HIR adds `infer_user_function_param_kinds(chunk)` and threads the
  result through `LowerCtx::for_function` as `external_kinds`.
- HIR `lower_function_body` emits a kind-appropriate `_ret_value`
  init (Number → 0.0, Bool → false, Nil → nil, Function → none).
- HIR `lower_call`'s arg-vs-param table accepts Bool↔Bool and
  Nil↔Nil.
- Codegen `ret_mlir_types` covers Bool/Nil; `emit_function`'s
  trailing return loads `i1` from the slot for those kinds.
- Anonymous function lowering passes a default `vec![Number; arity]`
  to `for_function` — the chunk-level pre-scan only sees top-level
  `FunctionDef` names.

## Out of Scope

- **Function-typed locals with Bool/Nil signatures** — the current
  `ValueKind::Function(arity)` lacks param/ret kind info, so passing
  a `pos`-style predicate to a `Function(1)` parameter would lose
  that information at the call-indirect site. A full signature is a
  Phase 2.5c+ extension.
- **Multi-call-site param refinement** (union type) — first call
  wins; conflicts are static errors. A union/widening rule is a
  later feature, gated on a heterogeneous-kind value path.
- **Inference for non-literal call args** (`local b = true; f(b)`
  should infer `f`'s param as Bool from `b`'s known kind). Doable
  with a more capable pre-scan but deferred — literal-arg coverage
  catches the common cases and lets the rest fall back to Number.
- **Closures, multi-return, table-typed signatures** — Phases
  2.5c / 2.5d / 2.6.
