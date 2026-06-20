# 0236. `<const>` / `<close>` Local Attributes — Parser + HIR

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M9 sub-ADR. Lua 5.4 §3.3.7 added two local-declaration attributes:

- `local x <const> = exp` — `x` is read-only for the rest of the scope.
- `local x <close> = exp` — `x` is also read-only AND invokes `x:__close()` when leaving scope.

This ADR lands the parser surface, AST representation, and the HIR enforcement for `<const>`. The `<close>` attribute parses and reaches HIR with no enforcement yet — the `__close` metamethod dispatch is M9-C scope. `<close>` Locals behave like ordinary Locals for the moment (no readonly enforcement, no cleanup dispatch).

## Scope (literal)

- ✅ Parser: `local NAME [<ATTR>]` (optionally repeated for multi-binding). The attribute is parsed as `Lt + Ident + Gt`. Trailing position after each name, before any `=`.
- ✅ AST: `StmtKind::Local { name, value, attr: Option<String> }` gains `attr`; `StmtKind::LocalMulti { names, values, attrs: Vec<Option<String>> }` gains a parallel `attrs` vector.
- ✅ HIR: `LocalInfo::is_const: bool` field. Set to `true` for any Local declared with `<const>`. The Local is also inserted into `readonly_locals` so the existing `lower_assign_target` reassignment rejection lights up.
- ✅ New `HirError::UnknownAttribute { name, offset }` variant rejects any attribute name other than `const` / `close` at HIR-lower time.
- ✅ `<close>` parses and reaches HIR; HIR records that the attribute was `close` but does NOT mark the Local readonly (Lua spec says `<close>` is also implicitly `<const>`, but the runtime hook is deferred so the readonly enforcement is too — defer the surface-visible semantics until the runtime lands).
- ❌ `__close` metamethod dispatch. M9-C scope.
- ❌ `<close>` readonly enforcement (Lua spec: `<close>` is implicitly const). Pinned to M9-C to ship the parsing infrastructure first.
- ❌ Multi-name shape with mixed attributes (`local a <const>, b = 1, 2`). Parses correctly via the per-name attr vector, but the HIR LocalMulti arm currently ignores `attrs`; const enforcement for multi-binding is a future micro-extension.
- ❌ Attribute checking at runtime. Static reject at HIR is sufficient — no runtime hook needed for `<const>`.

## Decision

### Parser shape

```text
local_stmt := 'local' name_list ['=' expr_list]
name_list  := name [attr] (',' name [attr])*
attr       := '<' Ident '>'
```

For each name in the loop, optionally consume `Lt + Ident + Gt`. The parser pushes the attribute (or `None`) into a parallel `attrs: Vec<Option<String>>`. When the names + values collapse into the single-name `StmtKind::Local`, the `attr` field carries the (popped) single attribute; otherwise `LocalMulti` keeps the full vector.

### HIR enforcement

```rust
// in lower(StmtKind::Local { name, value, attr }):
if let Some(a) = attr && a != "const" && a != "close" {
    return Err(HirError::UnknownAttribute { name: a.clone(), .. });
}
let id = declare_local_with_func_id(name, kind, func_id);
if attr.as_deref() == Some("const") {
    self.locals[id.0].is_const = true;
    self.readonly_locals.insert(id);
}
```

The `readonly_locals` set is the existing ADR-0035 ForNumeric loop-variable guard; inserting the const Local reuses the same rejection path. The user-visible error message is `loop variable '<name>' is read-only inside its 'for' body` which is a slight mis-naming for `<const>` but functionally correct; a future M9 cleanup can specialize the error.

### Why `<close>` parses without enforcement

Shipping the parser surface NOW means user code with `<close>` declarations compiles and runs (with the attribute effectively a no-op). When M9-C adds the runtime hook, the existing programs gain the cleanup behavior with no source rewrite. Conversely, gating parser acceptance on the runtime would force every `<close>`-using program to wait for M9-C — bad sequencing.

## Tests

`tests/phase4_m9_const_attribute.rs` (NEW, 5 e2e):

1. `local x <const> = 42; print(x)` → `"42"`.
2. `local x <const> = 1; x = 2` → `HirError::ReadOnlyAssign`.
3. `local r <close> = 7; print(r)` → `"7"` (parses + reads).
4. `local x <foo> = 1` → `HirError::UnknownAttribute`.
5. Nested user-fn body with `<const>` works.

## Test count delta

```
Step 0:  1561 (after ADR 0235)
C3 (impl + 5 e2e): 1561 → 1566
```

## References

- [Lua 5.4 §3.3.7](https://www.lua.org/manual/5.4/manual.html#3.3.7) — local attributes.
- [ADR 0035](0035-phase2-4b-repeat-until.md) — `readonly_locals` ForNumeric loop-variable origin (reused here).
- [Lua 5.4 §3.3.8](https://www.lua.org/manual/5.4/manual.html#3.3.8) — `__close` (M9-C scope).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M9 milestone.
