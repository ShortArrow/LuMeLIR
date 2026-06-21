# 0239. `__mode` Field — Surface Acceptance Pin + Weak-Table Deferral

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M10 sub-ADR. [ADR 0164](0164-gc-weak-tables.md) decided the weak-table strategy (`__mode = "k"` makes keys weak; `"v"` makes values weak; `"kv"` both). Like [ADR 0238](0238-gc-finalizer-field-pin.md) for `__gc`, the runtime semantics depend on Table DFS (M3-extended) — without keys / values participating in the mark phase, "clear weak references at GC time" has no observable behavior to test.

This ADR pins the surface that works today: `__mode` is a string-valued metatable field that round-trips through `mt.__mode` reads. The Lua 5.4 §2.5.4 spec semantics live in a future M10-stretch sub-ADR.

## Scope (literal)

- ✅ `mt.__mode = "k"` / `"v"` / `"kv"` storable on a metatable.
- ✅ Round-trip via `mt.__mode` reads back the assigned String.
- ✅ `setmetatable(t, mt)` with `mt.__mode` String value succeeds.
- ✅ `mt.__mode` participates in concat: `"mode=" .. mt.__mode` works.
- ❌ Weak-key references cleared at GC sweep time. M10-stretch.
- ❌ Weak-value references cleared at GC sweep time. M10-stretch.
- ❌ `__mode` ephemeron semantics (weak-key + strong-value-if-key-still-reachable, Lua 5.4 §2.5.4 special case). M10-stretch.
- ❌ Validation that `__mode` value is one of `"k"` / `"v"` / `"kv"`. Lua spec silently ignores invalid `__mode` strings; this matches today's behavior (no validation pass).
- ❌ Dynamic `__mode` change after `setmetatable`. Lua spec says `__mode` changes after the table is in use have undefined behavior; out of scope.

## Decision

No new code. `__mode` is a string field with no special parser / HIR / codegen handling. It flows through the existing Table machinery.

Future M10-stretch will:
- Add per-Table flag bits in the Table header (currently `gc_type_meta` has `has_outgoing_refs` only; a `weak_keys` / `weak_values` pair joins it).
- At `setmetatable` time, read `mt.__mode` and set the flag bits on the receiver Table.
- Hook the mark phase to skip walking weak-key / weak-value entries based on the flag bits.
- Hook the sweep phase to clear references whose payload-target is WHITE for weak-tagged slots.

## Tests

`tests/phase4_m10_mode_field_pin.rs` (NEW, 5 e2e):

1. `mt = { __mode = "k" }; print(mt.__mode)` → `"k"`.
2. Same for `"v"` and `"kv"`.
3. `setmetatable({}, mt)` with `mt.__mode` succeeds.
4. `__mode` participates in concat (`"mode=" .. mt.__mode` → `"mode=kv"`).

## M10 milestone close

ADRs 0238 + 0239 land the M10 surface acceptance at 2/2-3 minimum-viable. The runtime semantics for both `__gc` and `__mode` defer to M10-stretch which composes with M3-extended Table DFS.

Remaining M10-stretch work (deferred):

- `__gc` runtime registration + dispatch (paired with ADR 0163 §"Hook point" implementation).
- `__close` runtime hook on scope exit (M9-C scope — also blocked on M3-extended).
- `__mode` weak-key / weak-value clearing at sweep time.
- Per-Table flag bits in `gc_type_meta` for weak / has-finalizer.
- Resurrection-during-finalizer safety per ADR 0163 §"Resurrection-during-finaliser safety".

## Test count delta

```
Step 0:  1574 (after ADR 0238)
C3 (5 e2e): 1574 → 1579
```

## References

- [ADR 0156](0156-gc-architecture-v1.md) — GC architecture roadmap.
- [ADR 0164](0164-gc-weak-tables.md) — weak-table strategy.
- [ADR 0238](0238-gc-finalizer-field-pin.md) — sibling `__gc` pin + deferral.
- [ADR 0184](0184-gc-type-meta-size-guard.md) — `gc_type_meta` where the flag bits will land.
- [Lua 5.4 §2.5.4](https://www.lua.org/manual/5.4/manual.html#2.5.4) — weak tables / ephemerons spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M10 milestone close.
