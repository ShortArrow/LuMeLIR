# 0237. `<close>` Readonly Enforcement + Multi-Binding Attributes

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M9 sub-ADR. [ADR 0236](0236-const-close-attributes.md) landed the parser surface for `<const>` and `<close>` and the readonly enforcement for `<const>` only. Two limitations the user-visible spec gap demands fixing:

1. Per Lua 5.4 §3.3.8, `<close>` is implicitly `<const>` — the variable cannot be assigned after declaration. ADR 0236 explicitly deferred this so the parser surface could ship without the runtime `__close` hook.
2. The multi-binding shape `local NAME1 <ATTR>, NAME2 = ..., ...` parsed but the HIR `lower_local_multi` arm ignored the `attrs` vector entirely — every multi-bound Local was mutable regardless of attribute.

This ADR fixes both. The runtime `__close` metamethod dispatch remains M9-C scope (separate sub-ADR).

## Scope (literal)

- ✅ `<close>` is recognised by the same readonly enforcement path as `<const>` in both the single-binding (`StmtKind::Local`) and multi-binding (`StmtKind::LocalMulti`) lowering arms.
- ✅ Multi-binding shape consumes `attrs` per name. The parallel-binding branch (`values.len() == names.len()`) and the single-Call-expansion branch (`values.len() == 1`) both apply readonly per name.
- ✅ Attribute name validation runs once for the whole multi-binding statement, before any Local is declared, so an unknown attribute on the third name doesn't leave the first two in the locals table.
- ✅ M9-A test `close_attribute_parses_but_does_not_constrain_writes` is renamed `close_attribute_parses_and_reads` (the body shape stays — reads still work).
- ❌ Runtime `__close` metamethod dispatch on scope exit. M9-C scope (separate sub-ADR, requires GC integration + scope-exit hook in codegen).
- ❌ Cross-block flow for `goto` / `<close>` interaction. Future sub-ADR.
- ❌ Special-error-message specialisation. The user-visible error stays `ReadOnlyAssign` with the ForNumeric-flavoured message; a future cleanup ADR may specialise per source (loop var vs. `<const>` vs. `<close>`).

## Decision

### Single-binding match widens

```rust
if matches!(attr.as_deref(), Some("const") | Some("close")) {
    self.locals[id.0].is_const = true;
    self.readonly_locals.insert(id);
}
```

The two attribute names share the same enforcement; the runtime cleanup hook will be added at the `<close>` site in M9-C without affecting this path.

### Multi-binding propagates per name

`lower_local_multi` accepts the `attrs: &[Option<String>]` parameter and iterates it in lockstep with `names`. Both branches (parallel and single-Call-expansion) apply the same readonly insertion per declared Local. Attribute validation runs once up front:

```rust
for (_, attr) in names.iter().zip(attrs.iter()) {
    if let Some(a) = attr && a != "const" && a != "close" {
        return Err(HirError::UnknownAttribute { name: a.clone(), .. });
    }
}
```

## Tests

`tests/phase4_m9_close_readonly_multi.rs` (NEW, 5 e2e):

1. `local x <close> = 1; x = 2` → `ReadOnlyAssign`.
2. `local a <const>, b = 1, 2; a = 99` → `ReadOnlyAssign`.
3. `local a <const>, b = 1, 2; b = 99` succeeds (b is mutable).
4. `local a <foo>, b = 1, 2` → `UnknownAttribute`.
5. Both `<const>` and `<close>` Locals read correctly in the same scope.

`tests/phase4_m9_const_attribute.rs` (UPDATED): `close_attribute_parses_but_does_not_constrain_writes` → `close_attribute_parses_and_reads`.

## Test count delta

```
Step 0:  1566 (after ADR 0236)
C3 (impl + 5 new e2e + 1 renamed): 1566 → 1571
```

## References

- [ADR 0236](0236-const-close-attributes.md) — parser + single-binding `<const>` foundation.
- [Lua 5.4 §3.3.8](https://www.lua.org/manual/5.4/manual.html#3.3.8) — `<close>` semantics (cited spec for the implicit-const rule).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M9 milestone.
