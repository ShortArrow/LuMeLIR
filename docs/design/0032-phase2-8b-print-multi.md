# 0032. Phase 2.8b: Variadic `print(a, b, ...)`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.0 wired `print` as a fixed-arity (1) builtin. Stock Lua's
`print` is variadic, separating arguments with `\t` and ending with
`\n`:

```lua
print("x =", x, "y =", y)
print()
```

Both forms (zero-arg producing a bare newline, N-arg producing
tab-separated values) are immediate ergonomic wins for any
non-trivial diagnostic. This phase relaxes Print's arity, adds the
no-newline format-string siblings + `\t`/`\n` payload globals, and
funnels every `print` call through a single multi-arg loop that
closes with one trailing newline.

## Decision

### 1. HIR: Print becomes the one variadic builtin

Every other builtin keeps its fixed arity from `Builtin::arity()`.
`Builtin::Print` carves a dedicated branch in `lower_call` that
skips the fixed-arity check and accepts any `args.len() >= 0`.
`Builtin::Print::arity()` still returns `1`; the value is unused
for the variadic path but kept for code-base consistency
(documenting Lua's "typical" arity).

### 2. Codegen: four new module-top globals

| Symbol         | Payload  | Purpose                                |
|----------------|----------|----------------------------------------|
| `fmt_raw`      | `"%g"`   | Number print without trailing newline  |
| `fmt_str_raw`  | `"%s"`   | String/bool/nil print without newline  |
| `s_tab`        | `"\t"`   | Inter-argument separator               |
| `s_newline`    | `"\n"`   | Trailing newline after the last arg    |

The pre-existing `fmt = "%g\n"` and `fmt_str = "%s\n"` continue to
exist — `assert`'s diagnostic and a few other paths still want the
newline-attached forms.

### 3. Codegen: every print call loops

```text
if args.is_empty():
    printf("%s", "\n")
else:
    for i, arg in enumerate(args):
        if i > 0:
            printf("%s", "\t")
        printf("%g" or "%s", value-of-kind(arg))
    printf("%s", "\n")
```

- `emit_print_value_raw` is the per-arg dispatch — same kind
  switch as the old `emit_print_value` but with the `_raw` formats.
- `emit_print_literal(global_name)` emits `printf("%s", @<global>)`
  for the tab and newline payloads.

The result of `print(...)` in expression position is a placeholder
`f64 0.0`; previously the result was the (single) arg's value,
which existing tests didn't depend on, so the change is observable
only via inspection of unused intermediates.

### 4. Cleanup: four dead helpers removed

`emit_print_value`, `emit_print_nil`, `emit_printf_g`, and
`emit_print_bool` were the old single-arg path. With every print
call going through the loop, they became unused; the `_raw` family
fully replaces them. The behaviour is observably identical for
N=1.

### 5. Test suite migration

`hir::tests::lower_print_arity_mismatch_errors` previously asserted
that `print()` was an `ArityMismatch`. The new contract makes
`print()` legal (it emits a bare newline), so that unit test is
reframed as `lower_print_zero_arg_lowers_after_2_8b`.

## Alternatives Considered

- **Keep Print fixed-arity, add a new `printline(...)` variadic
  builtin**. Diverges from Lua. Rejected.
- **Introduce an `Arity` enum** (`Fixed(usize)` /
  `Variadic { min: usize }`). Cleaner long-term, but Print is
  currently the only variadic builtin; a one-line carve-out in
  `lower_call` is simpler and easier to audit. Adopt the enum if
  / when a second variadic builtin (e.g. `error(msg, ...)`,
  `string.format`) lands.
- **Reuse `fmt`/`fmt_str` and printf the `\n` ourselves at the
  end**. Would mean the inner per-arg printf calls would still
  emit a newline, producing one newline per arg. Wrong output.
- **Pass the format string as a parameter to a single helper**.
  Cleaner than the boolean-flag approach but at the cost of one
  extra `addressof` op per call site. Both are equivalent after
  LLVM optimization; the helper-pair form keeps the source diff
  contained.

## Consequences

- HIR: 5-line carve-out in `lower_call` for `Builtin::Print`'s
  arity check.
- Codegen:
  - Four new globals (`fmt_raw`, `fmt_str_raw`, `s_tab`,
    `s_newline`).
  - New `emit_print_value_raw` and `emit_print_literal` helpers.
  - `Callee::Builtin(Builtin::Print)` arm in `emit_expr` rewritten
    as the multi-arg loop above.
  - Four dead helpers removed (`emit_print_value`,
    `emit_print_nil`, `emit_printf_g`, `emit_print_bool`).
- Tests:
  - One HIR unit test reframed (`print()` now succeeds).
  - Nine integration tests in `phase2_8b_print_multi.rs` cover
    zero-arg, the regression for single-arg, two/three-arg with
    tab separators, the mixed-kind case, bool pair, user-function
    invocation, and the trailing-newline shape.

## Out of Scope

- **Generalising the variadic mechanism** to other builtins (e.g.
  `error(msg, level)`, `string.format`). The first one to need it
  is the right time to introduce an `Arity` enum.
- **Multi-result call expansion** — Lua's "the last call's results
  fill the remaining argument list" rule (e.g. `print(divmod(7,
  2))`). We have static multi-return slots from Phase 2.5d but
  no expansion-into-args path; defer.
- **`print` writing to a non-stdout stream**. `io.write` on a
  redirected `io.output()` is a separate stdlib feature.
