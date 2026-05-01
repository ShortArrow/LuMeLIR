# 0028. Phase 2.7e: `tonumber` Builtin with NaN Sentinel

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7c shipped `tostring`. Its inverse — `tonumber` — converts a
String to a Number, returning `nil` on parse failure in stock Lua.
Our type system doesn't yet support a "Number-or-Nil" return shape,
which is the natural model for a fallible conversion.

Rather than block on a heterogeneous return-kind feature, this
phase ships `tonumber` with a **NaN sentinel**: parse failure
yields `0/0` (the canonical quiet NaN). User code checks for it
with the standard Lua-level idiom `if x == x then ... end` (NaN is
the only IEEE 754 value that compares unequal to itself).

This is a deliberate divergence from Lua and is documented in the
ADR's "Out of Scope" notes; a future phase that introduces
heterogeneous return kinds (or a tagged union) can swap the
sentinel for `nil` without breaking any current observable
behaviour beyond the failure-detection idiom.

## Decision

### 1. `Builtin::ToNumber` joins the enum

`Builtin::ToNumber` becomes the third builtin variant after `Print`
and `ToString`. `from_name` recognises `"tonumber"`; `arity` is 1.
`infer_kind`'s `Callee::Builtin(Builtin::ToNumber)` arm returns
`ValueKind::Number`.

### 2. HIR-time arg-kind restriction

The shared builtin-arg check in `lower_call` already rejects
Function values (`FunctionUsedAsValue`). For `ToNumber` only, we
also reject Bool and Nil — there's no useful "to a number"
mapping for either kind in our subset. The accepted kinds are
exactly `Number` and `String`. Diagnostic message:
`tonumber number-or-string vs <kind>`.

### 3. Codegen dispatches by static kind

`emit_tonumber(value, kind)`:

- **`Number`** — identity (return `value` unchanged).
- **`String`** — three-operand variadic `sscanf(s, "%lf",
  &receiver)` into a stack-alloca'd f64 receiver. The call returns
  `1` on success, `0` on no-match. An `arith.cmpi Eq` against the
  i32 constant `1` produces an i1 success flag, and `arith.select`
  yields either the loaded receiver or `f64::NAN`.

A new `fmt_tonumber_lf` global holds the format string `"%lf\0"`.
`sscanf` is declared once at module top in
`emit_string_runtime_decls`, mirroring the variadic `snprintf` path
from Phase 2.7c.

`f64::NAN` materialises through melior's `FloatAttribute::new(_,
f64, f64::NAN)`. The IEEE 754 quiet-NaN bit pattern flows through
unchanged.

### 4. Failure detection is `x ~= x`

The user-level idiom for spotting a parse failure:

```lua
local n = tonumber(input)
if n == n then
  -- success, n is the parsed value
else
  -- failure (NaN)
end
```

This is the only currently-supported way to detect failure. When a
Number-or-Nil return shape lands, `tonumber` will swap to it and
the idiom will gain the more familiar `if n then ... end` form
without breaking the NaN-aware code (which will simply never see
NaN any more).

## Alternatives Considered

- **Return 0 on failure**. Hides errors; `0` is a perfectly valid
  parsed value. Rejected.
- **Return Nil on failure**, requiring a Number-or-Nil HIR return
  shape. The right long-term answer; out of scope for this phase
  because it touches the type system, MLIR signatures, and every
  consumer of `Callee::Builtin(Builtin::ToNumber)`'s return.
- **Trap on failure**. Diverges from Lua's "tonumber is fallible"
  contract; programs would need an explicit pre-validation step.
- **Two-result return**: `(value, ok)`. Useful but introduces a
  destructuring obligation at every call site. Rejected.
- **Use `strtod` instead of `sscanf`**. `strtod` reports failure
  via the `endptr` out-parameter — comparable boilerplate. `sscanf`
  is the more familiar dispatch and reuses the existing variadic
  call shape.

## Consequences

- HIR: `Builtin::ToNumber` variant; `infer_kind` arm; per-call
  arg-kind restriction in `lower_call`.
- Codegen: `Callee::Builtin(Builtin::ToNumber)` arm in `emit_expr`;
  new `emit_tonumber` helper; `fmt_tonumber_lf` global; `sscanf`
  added to `emit_string_runtime_decls`.
- Ten integration tests in `phase2_7e_tonumber.rs` cover integer
  / decimal / scientific / negative parse paths, NaN-sentinel
  failure detection, Number-identity, an arithmetic chain, plus
  three rejection paths (Bool, Nil, Function-kind args).

## Out of Scope

- **Returning `nil` on failure**. Pending a Number-or-Nil HIR
  return shape (or a tagged-union value model). When that lands,
  this builtin's semantic flips from sentinel to nil.
- **`tonumber(s, base)` with explicit numeric base**.
- **Locale / Unicode digits** in the input string — `sscanf "%lf"`
  uses the C locale's decimal grammar.
- **`assert(tonumber(s))` ergonomics** — needs `assert` (separate
  builtin, not yet in scope).
