# 0152. `string.format` (minimum-scope specifier set)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Last simple parallel side-track from ADR 0133's deferral table. Lua spec §6.4: `string.format(fmt, ...)` is the canonical printf-style formatter for Lua. Stock Lua supports the full printf-ish specifier set (`%d` / `%i` / `%u` / `%f` / `%e` / `%g` / `%s` / `%q` / `%x` / `%X` / `%o` / `%c` / `%%`, with width / precision / flags).

Our scope is the **minimum-useful subset** that covers the dominant idioms: `%d`, `%s`, `%f`, `%%`. The rest land as follow-up ADRs.

The format string must be a **static String literal** at the call site — runtime-computed format strings need an interpreter rather than a codegen specialiser; deferred.

## Scope (literal)

- ✅ `string.format(fmt_literal, ...)` where `fmt_literal` is an HIR `Str` literal.
- ✅ Specifiers: `%d` (Number → integer print), `%f` (Number → float print, `%g`-style), `%s` (String → boxed-object data ptr passed to `printf %s`), `%%` (literal `%`).
- ✅ Result: boxed String object (ADR 0112 ABI).

Out of scope:

- ❌ `%i` / `%u` / `%e` / `%g` / `%q` / `%x` / `%X` / `%o` / `%c` and any width / precision / flag suffixes.
- ❌ Runtime-computed format string.
- ❌ TaggedValue arg (only static kinds).
- ❌ More than 16 arguments (arbitrary but bounded; matches the C variadic stack frame budget we already use elsewhere).

## Decision

### HIR

New `Builtin::StringFormat` variant, arity `(1, usize::MAX)`, first param `String`, rest variadic. `expected_param_kind` returns:

| arg index | Required kind |
|---|---|
| 0 | `String` (the format literal) |
| ≥ 1 | inferred from the specifier at the corresponding position in the parsed format string |

`lower_namespace_builtin_call` for `string.format`:

1. Lower all args.
2. Require `lowered_args[0].kind == HirExprKind::Str(s)`. Otherwise → `TypeMismatch` ("string.format requires a static format string").
3. Parse `s` to extract the specifier sequence. Each `%d`/`%f` requires Number; `%s` requires String; `%%` consumes no arg.
4. Validate `args.len() - 1 == specifier_count` (`ArityMismatch` otherwise).
5. For each non-format arg, validate its inferred kind matches.

Returns `String`.

### Codegen

`emit_expr` `Callee::Builtin(Builtin::StringFormat)` arm:

1. Re-parse the format string (the HIR arg 0 is the same `Str` literal).
2. Build a scratch buffer of fixed size (256 bytes — bounded printf frame).
3. Compute the format string for `snprintf` — for `%s` args, the data ptr from boxed String objects; for `%d`/`%f`, the f64 / `i64`-from-f64 directly.
4. Emit `snprintf(buf, size, fmt_cstr, args...)` via the variadic C call ABI we already use for `tostring(Number)`.
5. Wrap the result in a boxed String object via `emit_string_obj_from_bytes` (ADR 0112).

The format C-string is a module-level `s_format_<hash>` global registered at emit time (mirror `s_typename_*` pattern).

Width / precision / flag suffixes in the format string trigger an HIR-level `TypeMismatch` ("unsupported format spec").

## Alternatives considered

- **Full printf surface.** Rejected — width / precision / flags / `%g` exponent rules / locale specifics balloon the spec. Defer to per-specifier follow-ups.
- **Runtime format-string parsing.** Rejected — needs a runtime parser + dispatch table. Static is the right cut for Phase 2.
- **Treat format as raw passthrough to `snprintf`**. Rejected — `%s` against our boxed String objects without extracting the data ptr produces garbage.
- **TaggedValue args.** Rejected — adds a tag check per arg; defer with the broader TaggedValue stdlib-arg ABI.

## Consequences

**Positive**
- The dominant `string.format` use case works: `print(string.format("%s = %d", name, val))`.
- Codegen reuses `emit_string_obj_from_bytes` from ADR 0112.
- HIR-level specifier parser stays in `src/hir/mod.rs` near `lower_namespace_builtin_call`.

**Negative**
- Unsupported specifiers / width / precision reject at HIR time. Documented as a known limitation.
- Up-to-16-arg limit. Bounded but visible.

**Locked in until superseded**
- Static format string only.
- `%d` / `%f` / `%s` / `%%` subset.
- 16-arg cap.
- 256-byte scratch buffer.

## Documentation updates

- [x] §4 LIC — new `LIC-string-format-1`.
- [x] §7 — closes `string.format` open item.
- [x] §8 — adds 0152.

## Test count delta

```
Step 0:   1360 (after ADR 0151)
C2 (6 e2e Red Day 0):  1360 → 1360
C3 (impl): 1360 → 1366
```

## Critical files

- `src/hir/ir.rs`: `Builtin::StringFormat` variant + `from_namespace_method` arm + `name` + `arity` + `ret_kinds` + `expected_param_kind`.
- `src/hir/mod.rs`: `lower_namespace_builtin_call` `StringFormat` arm — static-format check, specifier parser, arg-kind validation. Helper `parse_format_specifiers(&str) -> Result<Vec<FormatSpec>, HirError>`.
- `src/codegen/emit.rs`: `Callee::Builtin(StringFormat)` emit arm. Helper `emit_string_format_runtime`.
- `tests/phase2_string_format.rs` (NEW) — 6 e2e (1 per specifier + arity + mixed).
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Format string parser rejects valid stock-Lua format | Documented limitation; e2e tests pin the supported subset only. |
| 256-byte scratch overflows for long format outputs | snprintf truncates at the buffer boundary; we don't grow yet. Future ADR may bump or grow. |
| `%s` against a non-NUL-terminated String object | ADR 0112's compat NUL terminator covers this. |
| Variadic call ABI mismatch with mlir-translate | We already use the same shape in `emit_tostring` (Number arm); the ABI is established. |

## Future work

- Full Lua format specifier set (`%g` / `%e` / `%i` / `%u` / `%x` / `%X` / `%o` / `%q` / `%c`).
- Width / precision / flag suffixes.
- Runtime format string.
- TaggedValue args.
- Dynamic scratch buffer growth.

## References

- [ADR 0103](0103-phase2-stdlib-string-begin.md) — `string` namespace dispatch.
- [ADR 0112](0112-phase2-string-abi-refactor.md) — boxed String object ABI.
- [ADR 0113](0113-phase2-string-char.md) — variadic Number arg specifier (`expected_param_kind` precedent).
- Lua 5.4 reference manual §6.4 — `string.format`.
