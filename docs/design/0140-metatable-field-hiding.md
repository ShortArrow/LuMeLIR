# 0140. `__metatable` Field Hiding

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Lua spec §6.1 documents `__metatable` as a **protection field**:

- `getmetatable(t)` returns `mt.__metatable`'s value (if present and non-nil) instead of the metatable itself.
- `setmetatable(t, ...)` raises an error if `t`'s existing metatable carries a non-nil `__metatable` field.

Today (post-[ADR 0138](0138-setmetatable-nil-clear.md)), our `getmetatable` returns the raw metatable pointer and `setmetatable` overwrites unconditionally. The `__metatable` field is silently ignored. This is the last Tier 1 sweep gap on `plans/zany-giggling-cat.md`.

## Scope (literal)

**Both `getmetatable` and `setmetatable` honour `__metatable` field hiding.** Hash-key only (`"__metatable"` is a String literal, takes the static-kind path).

Out of scope:

- ❌ `rawset` / `rawget` interaction with `__metatable` — they already bypass the metatable chain. No change needed (and tests pin this).
- ❌ TaggedValue probe through `__metatable` chains — `__metatable` itself is a single-hop probe; no chain semantics in spec.

## Decision

### New runtime globals

In `src/codegen/emit.rs` `emit_module_init`:

- `s_metatable_field_name` — `"__metatable\0"` C-string (mirrors `s_metatable_newindex_field_name` from ADR 0135).
- `s_setmetatable_protected` — Lua spec error message `"cannot change a protected metatable"`.

### `emit_getmetatable_runtime` extension

After loading `mt` at offset 32, dispatch on `mt == null`:

1. Null → write Nil to `out_slot` (unchanged from ADR 0134).
2. Non-null → probe `mt["__metatable"]` via `emit_hash_lookup_into_tagged_slot(NilOnMissing)` into a tmp tagged slot:
   - If the probe result tag is `TAG_NIL` → write `mt` as Table-tagged to `out_slot` (existing path).
   - If non-Nil → raw 16-byte copy of the probe-result slot into `out_slot` (`emit_copy_tagged_slot_16b` from ADR 0139).

### `emit_setmetatable_runtime` extension

Before storing the new metatable ptr at offset 32:

1. Load `cur_mt` at offset 32.
2. If `cur_mt != null`:
   - Probe `cur_mt["__metatable"]` via `emit_hash_lookup_into_tagged_slot(NilOnMissing)`.
   - If the probe-result tag is **not** `TAG_NIL` → trap with `s_setmetatable_protected`.
3. Store the new metatable ptr (unchanged from existing path).

Returns `t_ptr` (Lua spec).

## Alternatives considered

- **Cache the `__metatable` field at install time** (write a side flag in the table header instead of probing at every `getmetatable` / `setmetatable` call). Rejected — the probe is cheap (single hash bucket), and a side flag drifts when the metatable is mutated post-install (`mt.__metatable = x` after `setmetatable(t, mt)`).
- **Skip the protection at `setmetatable`** (only honour it at `getmetatable`). Rejected — half-spec compliance is worse than none; a user who sets `__metatable` to defend an object would be surprised that `setmetatable(t, nil)` from outside still clears it.
- **Treat `__metatable = false` as protected (Lua reference behaviour)**. Accepted implicitly — the probe checks `TAG_NIL` only; any non-Nil tag (including Bool false) trips protection on `setmetatable` and is returned on `getmetatable`. Matches Lua reference impl.

## Consequences

**Positive**
- `setmetatable` / `getmetatable` now match Lua spec §6.1.
- Users can defend custom objects against external metatable replacement, the canonical pattern for "sealed" Lua APIs.

**Negative**
- Every `setmetatable(t, mt2)` call now performs an extra hash probe on `t`'s existing metatable (when present). Negligible — single bucket lookup on a typically tiny metatable.
- Every `getmetatable(t)` call now performs an extra hash probe on `t`'s metatable (when present). Same.

**Locked in until superseded**
- Probe is per-call (no caching). Future ADR can add a header bit if profiling shows it matters.

## Documentation updates

- [x] §1–§3 — **no change**.
- [x] §4 LIC consolidation — new resolved entry `LIC-metatable-field-hiding-1`.
- [x] §7 open questions — closes `__metatable` field hiding open item.
- [x] §8 ADR index — adds 0140.

## Test count delta

```
Step 0:   1305 (after ADR 0139)
C2 (5 new e2e Red Day 0): 1305 → 1305
C3 (impl):                1305 → 1310 (all green)
```

## Critical files

- `src/codegen/emit.rs`:
  - 2 new diagnostic globals (`s_metatable_field_name`, `s_setmetatable_protected`).
  - `emit_getmetatable_runtime` — extended with the `__metatable` probe + raw-copy branch.
  - `emit_setmetatable_runtime` — extended with the protection check + trap.
- `tests/phase2_6plus_metatable_field_hiding.rs` (NEW) — 5 e2e.
- `docs/design/tagged-semantics.md` — §4 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| `rawset` / `rawget` regress on `__metatable` field handling | They already bypass metatable chains by construction; tests 4 / 5 pin this. |
| `setmetatable(t, nil)` (ADR 0138 clear) regresses when `t` has `__metatable` | The protection check fires before the store; `setmetatable(t, nil)` should ALSO trap when `__metatable` is set. Test 3 pins. |
| Probe runs even when no metatable installed | The null-mt fast path is unchanged — the probe is gated on `cur_mt != null`. |

## Future work

- Header-bit cache for `__metatable` presence (only if profiling shows the probe matters).

## References

- [ADR 0134](0134-metatables-index-read.md) — `emit_getmetatable_runtime` and `emit_setmetatable_runtime` initial shape.
- [ADR 0138](0138-setmetatable-nil-clear.md) — `setmetatable(t, nil)` clear; this ADR adds the protection check on top.
- [ADR 0139](0139-taggedvalue-key-newindex-wiring.md) — `emit_copy_tagged_slot_16b` reused for the raw slot copy.
- Lua 5.4 reference manual §6.1 — `__metatable` field semantics.
