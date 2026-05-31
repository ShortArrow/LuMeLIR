# 0137. `rawequal` and `rawlen` Builtins (Table-only)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

[ADR 0136](0136-raw-set-get-builtins.md) landed `rawset` and `rawget`, completing the read/write escape-hatch pair for hash-key Index/IndexAssign. Lua spec §6.1 documents two more `raw*` builtins — `rawequal(v1, v2)` and `rawlen(v)` — that bypass the `__eq` and `__len` metamethods respectively.

Neither `__eq` nor `__len` exists in the project yet, so functionally `rawequal` is identical to `==` and `rawlen` is identical to `#` today. The value of this ADR is **Lua spec parity surface** + **future-proofing**: when the per-op metamethod ADRs land, `rawequal` and `rawlen` will already exist as the canonical escape hatches and won't need to be retrofitted into the dispatch chains.

Per the Tier 1 sweep plan, this is the smallest principled cut after 0136: same builtin-family shape, no new chokepoint, no ABI shift.

## Scope (literal)

**`rawequal` and `rawlen`, Table operand only.** Everything else is explicitly out of scope:

- ❌ `rawequal(v1, v2)` with non-Table operands — for Number / String / Bool / Function, ordinary `==` already returns the same answer (no metamethod in scope today). Deferred until `__eq` lands and the asymmetry becomes observable.
- ❌ `rawlen(s)` for String — `#s` already returns string length directly (no `__len` in scope today). Same reasoning.
- ❌ TaggedValue operands — same scope restriction as ADR 0136.
- ❌ `__eq` / `__len` metamethod dispatch — separate per-op ADRs (Tier 2 / Tier 3 on the roadmap).

## Decision

### HIR

Two new `Builtin` variants in `src/hir/ir.rs`:

- `Builtin::RawEqual` — arity `(2, 2)`, params `[Table, Table]`, returns `Bool`.
- `Builtin::RawLen` — arity `(1, 1)`, params `[Table]`, returns `Number`.

`Builtin::from_name` maps bare identifiers `rawequal` and `rawlen` to the variants (same pattern as `rawset` / `rawget` from ADR 0136).

`lower_builtin_call` enforces kind constraints alongside the existing `RawSet | RawGet` checks:

- `rawequal`: both args must be `Table`. Non-Table operands trip `HirError::TypeMismatch` with a "table" message.
- `rawlen`: arg 0 must be `Table`. Non-Table trips the same.

### Codegen

New emit arms in `src/codegen/emit.rs` under the `Callee::Builtin(b)` match in `emit_expr`:

- `Builtin::RawEqual`:
  1. Lower the two Table-ptr args to `lhs_ptr`, `rhs_ptr`.
  2. `llvm.ptrtoint` both to i64.
  3. `arith::cmpi(Eq, lhs_i, rhs_i)` — yields the Bool i1 result.
  4. Return the i1 value.

- `Builtin::RawLen`:
  1. Lower the Table-ptr arg to `t_ptr`.
  2. Load `length: i64` at offset `TABLE_OFF_LEN` (= 0).
  3. `arith::sitofp` i64 → f64 (Number kind contract).
  4. Return the f64 value.

No new helpers, no new globals, no new traps. Both arms are 5-10 LOC each.

## Alternatives considered

- **Bundle into ADR 0136 retroactively** — Rejected: ADR-per-decision violation; 0136 was already approved and shipped.
- **Accept all kinds for `rawequal` / `rawlen` now** — Rejected: without `__eq` / `__len`, the multi-kind dispatch carries zero observable benefit and adds runtime tag-check overhead. Wait until the metamethod ADR makes the asymmetry visible.
- **Skip this ADR entirely (no observable behavior)** — Rejected: the surface is canonical Lua and the cost is negligible (~50 LOC across HIR + codegen). Locking in the builtin name now prevents a later collision with user-defined `rawequal` / `rawlen` globals once the metamethods arrive.

## Consequences

**Positive**
- Lua spec parity surface advances — `rawequal` / `rawlen` are no longer absent from the global namespace.
- Future `__eq` / `__len` ADRs land into a project where the escape hatches already exist — no retrofit churn.
- Zero new heap-alloc, zero ABI shift, zero new chokepoint helpers.

**Negative**
- Adds two HIR variants whose codegen is functionally redundant with existing `==` / `#` today. The redundancy is intentional (forward-compat) but does mean the new e2e tests are coverage rather than new behavior.

**Locked in until superseded**
- Table-only scope for both builtins. String / Number / Bool / Function `rawequal` and String `rawlen` arrive with the `__eq` / `__len` metamethod ADRs (whichever lands first will decide the broader operand surface).

## Documentation updates

[`docs/design/tagged-semantics.md`](tagged-semantics.md) is the SoT for the TaggedValue runtime model (ADR 0068). This ADR touches:

- [x] §1 slot layout — **no change**.
- [x] §2 producer / source taxonomy — **no change** (both builtins return concrete kinds, not TaggedValue).
- [x] §3 consumer coverage matrix — **no change**.
- [x] §4 LIC consolidation — new resolved entry `LIC-raw-equal-len-1`.
- [x] §5 runtime tag invariants — **no change**.
- [x] §7 open questions — closes the `rawequal` / `rawlen` open items; `__eq` / `__len` remain open per their separate ADRs.
- [x] §8 ADR index — adds 0137 to the chronological table.

## Test count delta

```
Step 0:   1295 (1287 + 8 raw-builtins green)
Commit C2 (6 new e2e Red Day 0):  1295 → 1295 (existing green; 6 new red)
Commit C3 (HIR + codegen impl):   1295 → 1301 (all green)
```

## Critical files

- `src/hir/ir.rs` — 2 new `Builtin` variants + `from_name` + `arity` + `name` + `ret_kinds` + `param_kinds_for_arity` arms.
- `src/hir/mod.rs` — `infer_kind` arm + per-builtin kind checks in `lower_builtin_call`.
- `src/codegen/emit.rs` — `Callee::Builtin(RawEqual | RawLen)` emit arms.
- `tests/phase2_6plus_raw_equal_len.rs` (NEW) — 6 e2e tests.
- `docs/design/tagged-semantics.md` — §4 / §7 / §8 updates.

## Risks

| Risk | Mitigation |
|---|---|
| `rawequal` returns wrong answer when called with two distinct-but-equal tables | Lua spec: distinct tables are never equal regardless of contents. Ptr-equality is the spec-correct semantics. Test 1 pins. |
| `rawlen` returns array length when the table has no array part | Existing `#t` already returns 0 for empty array part; `rawlen` reuses the same field. Test 4 pins. |
| Non-Table operand silently coerced | HIR `TypeMismatch` reject at lower time. Tests 5/6 pin via expected non-zero exit / compile error. |

## Future work

- `__eq` metamethod (Tier 2 on the roadmap) — broadens `rawequal`'s operand surface.
- `__len` metamethod (separate small ADR) — broadens `rawlen`'s operand surface.
- String `rawlen` — folds into the `__len` ADR.

## References

- [ADR 0058](0058-phase2-6b-hash-keys.md) — table header layout (length at offset 0).
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table; `__eq` / `__len` rows.
- [ADR 0136](0136-raw-set-get-builtins.md) — sibling `rawset` / `rawget`; same builtin family + HIR pattern.
- Lua 5.4 reference manual §6.1 — `rawequal` / `rawlen` semantics.
