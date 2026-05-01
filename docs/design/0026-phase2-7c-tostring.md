# 0026. Phase 2.7c: `tostring` Builtin and Concat Auto-Coercion

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7b made `..` work for `String..String` but rejected mixed
operands. Stock Lua silently converts numbers, booleans, and `nil`
to strings before concat, so a typical pattern like

```lua
print("count: " .. n)
```

still failed with `TypeMismatch`. The same conversion is exposed
through the `tostring(x)` standard-library function, which is the
ergonomic way to turn any value into a string.

This phase introduces `tostring` as a HIR builtin, wires the
runtime through libc `snprintf`, and uses it as the lowering target
for non-String operands of `..`.

## Decision

### 1. `Builtin::ToString` joins the enum

`Builtin::ToString` becomes the second variant of `Builtin` (after
`Print`). `from_name` recognises `"tostring"`; `arity` is 1.

`infer_kind`'s `Callee::Builtin(Builtin::ToString)` arm returns
`ValueKind::String`, so a `tostring(x)` call slots cleanly into the
existing String-typed expression world.

`lower_call`'s builtin path already accepts Number/Bool/Nil/String
args and rejects `Function(_)`. `tostring` reuses that check
verbatim — Function values are still surfaced as
`HirError::FunctionUsedAsValue`.

### 2. Concat auto-coerces via the new helper

A small `coerce_to_string(expr, kind, offset)` helper lowers
non-String values into `Call(Builtin::ToString, [expr])` at HIR
time. `BinOp::Concat`'s arm now invokes it on both sides:

| Operand kind  | Result                                |
|---------------|---------------------------------------|
| `String`      | unchanged                             |
| `Number`      | wrapped in `tostring(...)` call       |
| `Bool`        | wrapped in `tostring(...)` call       |
| `Nil`         | wrapped in `tostring(...)` call       |
| `Function(_)` | `TypeMismatch`                        |

The desugar happens once at HIR time so codegen sees the existing
`String..String` shape and the runtime path from Phase 2.7b
(malloc + memcpy) needs no further changes.

### 3. Codegen: per-kind `emit_tostring`

`emit_tostring(value, kind)` dispatches by static kind:

- **`String`** — identity (return the same `ptr`).
- **`Bool`** — `llvm.select` between `s_true` / `s_false` ptrs.
- **`Nil`** — `addressof @s_nil` (existing global).
- **`Number`** — `malloc(32)` + `snprintf(buf, 32, "%.14g", n)`.
  The `%.14g` format matches Lua's spec for `tostring(number)`,
  trimming trailing zeros while keeping enough precision for any
  IEEE 754 double. The buffer leaks intentionally (no GC,
  consistent with Phase 2.7b concat).

A new `fmt_tostring_g` global holds the `"%.14g"` format string.
`snprintf` is declared once at module top via the existing
`emit_string_runtime_decls`, mirroring the printf path's
`var_callee_type` attribute pattern for variadic calls.

### 4. Function-value rejection stays in HIR

`tostring(f)` for a Function-kind operand never reaches codegen —
the existing builtin-arg check in `lower_call` rejects it as
`FunctionUsedAsValue`. The codegen `emit_tostring` arm for
`Function(_)` is therefore `unreachable!()`.

## Alternatives Considered

- **Keep `..` strict and require explicit `tostring(x)`** at every
  use site. Faithful to a stricter type system but diverges from
  Lua. Rejected — the auto-coerce is one of Lua's defining
  ergonomic features.
- **Stack-allocate the `tostring(number)` buffer** instead of
  `malloc`. Would eliminate the leak but escapes the function
  boundary the moment the result flows into a variable; alloca'd
  buffers can't outlive the callee frame. Rejected.
- **Use `%g` instead of `%.14g`**. `%g` defaults to 6 significant
  digits and would lose precision for round-tripping (`tostring(1
  / 3)` would print `0.333333` instead of `0.33333333333333`). The
  Lua spec mandates `%.14g`; we follow.
- **Implement `tonumber` in the same phase.** The natural inverse
  but it returns `nil` on parse failure, which needs a heterogeneous
  return shape (Number-or-Nil) we don't model yet. Defer.

## Consequences

- HIR: `Builtin::ToString` variant + `infer_kind` arm; new
  `coerce_to_string` helper; `BinOp::Concat` lowering uses it on
  both operands.
- Codegen: `Callee::Builtin(Builtin::ToString)` arm in `emit_expr`;
  new `emit_tostring` helper; `fmt_tostring_g` global; `snprintf`
  added to `emit_string_runtime_decls`.
- Twelve integration tests in `phase2_7c_tostring.rs` cover the
  six operand kinds for direct `tostring(...)` calls, four
  auto-coerce concat paths (number, bool, nil, lhs-of), one usage
  inside a user function returning String, and one rejection path
  (Function-kind `tostring`).

## Out of Scope

- **`tonumber`** — needs heterogeneous Number-or-Nil return.
- **GC / refcount of `tostring`-produced buffers**. They leak
  under the same model as concat; future GC phase covers both.
- **Lexicographic comparison** (`<`/`>` on strings) — Phase 2.7d.
- **Function-value `tostring`** producing addresses or symbol
  names. No use case in the current scope.
