# 0214. Subtype-Distinguished `print` for `HirExprKind::Integer`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-17
- **Deciders:** ShortArrow

## Context

Sixth M1 sub-ADR. ADRs 0209-0213 wired `HirExprKind::Integer(i64)` end-to-end at the kind layer (parser → HIR → constant fold) but kept Phase B silent demotion at codegen: every Integer leaf was lowered via `i64 const → sitofp → f64`, then `print` formatted it with `%g`. Two user-visible defects:

1. `print(42)` printed `42` only by chance — `print(0.5 + 0.5)` correctly prints `1`, but `print(math.maxinteger)` prints `9.22337e+18` (f64 cannot represent `i64::MAX`).
2. `print(1 + 2)` printed `3` only because `%g` strips the trivial fraction; mixed integer arithmetic that pushed past `2**53` would lose precision.

Both ADR 0211 §Future work and ADR 0212 §Scope explicitly deferred this to its own ADR.

## Scope (literal)

- ✅ `print` Print arm checks `HirExprKind::Integer(i)` as the first arg-kind discriminator. On match, emits `i64 const` + `printf("%lld\0", const)` directly — no `sitofp`, no `%g`.
- ✅ Composes with all M1 sub-ADRs: `print(math.maxinteger)`, `print(1 + 2)`, `print(10 // 3)`, `print(7 & 5)` all print without `.0` and without precision loss.
- ✅ New `fmt_lld_raw` cstr global = `"%lld\0"`. Sibling of `fmt_raw` (`"%g\0"`); the trailing newline / tab is appended by the outer Print arm.
- ❌ `tostring(integer-literal)`. Separate format pipeline (`%.14g`); subtype-aware tostring is a future sub-ADR.
- ❌ Runtime Integer-tagged TaggedValue slot. `print(Local x)` where `x = 1` still routes through the f64 `fmt_raw` path because slot subtype tracking is Phase C scope.
- ❌ `io.write(integer)`. The io.write arm has its own dispatch chain; mirroring the fast path there is a trivial future delta but out of scope here.
- ❌ String concatenation of Integer (`tostring`-shaped). Concat goes through a separate `fmt_tostring_g` path.

## Decision

`src/codegen/emit.rs`:

1. Register a new cstr global at module init:
   ```rust
   emit_cstr_global(context, module, i8_type, "fmt_lld_raw", "%lld\0", loc);
   ```
2. In `Callee::Builtin(Builtin::Print)` arm, at the top of the per-arg loop (before `infer_kind` and before the TaggedValue / Index / Call dispatch checks):
   ```rust
   if let HirExprKind::Integer(i) = &a.kind {
       let i64_const = block.append_operation(arith::constant(
           context, IntegerAttribute::new(types.i64, *i).into(), loc,
       )).result(0).unwrap().into();
       let fmt_ptr = emit_addressof(context, block, "fmt_lld_raw", types, loc);
       emit_printf(context, block, fmt_ptr, i64_const, types, loc);
       continue;
   }
   ```

`emit_printf`'s variadic descriptor accepts any-type variadic args, so passing i64 directly is sound (printf consumes `%lld` as `long long`, which matches `i64` on every supported target).

## Tests

`tests/phase4_print_integer_precision.rs` (NEW, 5 e2e):

1. `print(42)` → `"42"` (no `.0` fraction).
2. `print(math.maxinteger)` → `"9223372036854775807"` (precision preserved).
3. `print(math.mininteger)` → `"-9223372036854775808"`.
4. `print(1 + 2)` → `"3"` (folded Integer prints clean).
5. `print(1, 2, 3)` → `"1\t2\t3"` (multi-arg + tab separator composes).

## Test count delta

```
Step 0: 1473 (after 0213 land)
C3 (impl + 5 e2e): 1473 → 1478
```

## References

- [Lua 5.4 §6.1 `print`](https://www.lua.org/manual/5.4/manual.html#pdf-print)
- [ADR 0209](0209-integer-ast-hir-variant.md) — `HirExprKind::Integer`.
- [ADR 0212](0212-math-integer-constants.md) — defers maxinteger precision to this ADR.
- [ADR 0213](0213-integer-binop-constant-folding.md) — folded Integer feeds the print arm.
- [`docs/notes/roadmap-revision-2026-06-16.md`](../notes/roadmap-revision-2026-06-16.md) — M1 milestone sixth sub-ADR.
