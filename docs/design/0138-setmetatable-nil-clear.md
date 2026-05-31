# 0138. `setmetatable(t, nil)` — Metatable Clear

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0134](0134-metatables-index-read.md) installed `setmetatable(t, mt)` as a Table-only builtin: both args must be `ValueKind::Table`. The Lua spec §6.1 form `setmetatable(t, nil)` — which **clears** `t`'s metatable, restoring raw `__index` / `__newindex` semantics — was deferred as out-of-scope.

With the metatables read+write+raw families landed (ADRs 0134 / 0135 / 0136 / 0137), the missing nil-clear is the smallest remaining gap in the `setmetatable` surface. Per the Tier 1 sweep on `plans/zany-giggling-cat.md`, this is Step B.

## Scope (literal)

**`setmetatable(t, nil)` accepted at HIR.** Writes a null pointer at `t`'s header offset 32 (`TABLE_OFF_METATABLE`), reverting subsequent Index / IndexAssign on `t` to the raw hash path.

Out of scope:

- ❌ `setmetatable(t, mt)` with a TaggedValue second arg — the dynamic Table-vs-Nil dispatch needs runtime tag-check + branch, not in this ADR.
- ❌ `__metatable` field hiding — separate small ADR (Tier 1 Step Q).
- ❌ Shared metatable + GC interaction — gated on the GC strategy ADR.

## Decision

### HIR

`lower_builtin_call` widens the `SetMetatable` arg 1 check from "must be Table" to "must be Table or Nil". Arg 0 is unchanged (still Table-only).

`Builtin::SetMetatable` keeps arity `(2, 2)` and `ret_kinds = [Table]` (still returns `t`).

### Codegen

The existing `Callee::Builtin(Builtin::SetMetatable)` arm in `src/codegen/emit.rs` dispatches on `infer_kind(&args[1], ...)`:

- `ValueKind::Table` → existing path (`emit_setmetatable_runtime` stores `mt_ptr` at offset 32).
- `ValueKind::Nil` → new path: emit a null `!llvm.ptr` constant and store at offset 32. Return `t_ptr` (Lua spec — `setmetatable` returns its first argument).

The null store uses `llvm.mlir.zero` to produce a typed null ptr.

## Alternatives considered

- **TaggedValue arg 1 with runtime dispatch.** Rejected for this ADR — adds a runtime tag-check branch and ties the nil-clear into the broader TaggedValue-metatable dispatch that the future `__index = Function` / `__call` ADRs will need anyway. Wait for that ADR to subsume it.
- **A separate `clearmetatable(t)` builtin.** Rejected — non-canonical, would surprise Lua users.
- **HIR-level constant-folding of `setmetatable(t, nil)` into a no-op-and-clear.** Rejected — keeps the call shape uniform; codegen is the right layer for the Nil branch.

## Consequences

**Positive**
- `setmetatable` surface matches Lua spec for the static-Nil case.
- The chain ADR 0134's `metatable_ptr == null` fast-path is now reachable from user code, not just `setmetatable`-never-called.

**Negative**
- Slight HIR surface widening (one extra kind accepted at arg 1). The error message at the remaining rejection ("non-table, non-nil arg") needs updating.

**Locked in until superseded**
- Static-Nil only. TaggedValue arg 1 lands with the runtime-tag dispatch ADR.

## Documentation updates

- [x] §1–§5 — **no change**.
- [x] §4 LIC consolidation — new resolved entry `LIC-setmetatable-nil-clear-1`.
- [x] §7 open questions — closes the `setmetatable(t, nil)` open item.
- [x] §8 ADR index — adds 0138.

## Test count delta

```
Step 0:   1301 (1295 + 6 raw-equal-len)
C2 (3 new e2e Red Day 0): 1301 → 1301 (existing green; 3 new red)
C3 (HIR + codegen impl):  1301 → 1304 (all green)
```

## Critical files

- `src/hir/mod.rs` — `lower_builtin_call` widens SetMetatable arg-1 accepted kinds to `{Table, Nil}`.
- `src/codegen/emit.rs` — `Callee::Builtin(SetMetatable)` arm dispatches on arg-1 kind; new Nil branch stores typed null ptr.
- `tests/phase2_6plus_setmetatable_nil.rs` (NEW) — 3 e2e tests.
- `docs/design/tagged-semantics.md` — §4 / §7 / §8 updates.

## Risks

| Risk | Mitigation |
|---|---|
| Nil clear silently fails to restore raw read semantics | Test 1: install mt with `__index`, clear it, observe raw nil on read. |
| `setmetatable(t, nil)` mistakenly returns nil instead of `t` | Lua spec: returns `t`. Test 2 chains `setmetatable(setmetatable(t, mt), nil)` and asserts the final ptr equals `t`. |
| Existing `setmetatable(t, mt)` callers regress | The Table arm code path is untouched; Nil branch is a new arm. The 11 ADR-0134 green tests act as the regression net. |

## Future work

- TaggedValue arg 1 runtime tag-check + nil-clear dispatch (subsumed by the future `__index = Function` / GC-aware ADR).
- `__metatable` field hiding (Tier 1 Step Q).
- Shared metatable + GC interaction (gated on the GC strategy ADR).

## References

- [ADR 0134](0134-metatables-index-read.md) — original SetMetatable that this ADR widens.
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table.
- Lua 5.4 reference manual §6.1 — `setmetatable(t, nil)` clear semantics.
