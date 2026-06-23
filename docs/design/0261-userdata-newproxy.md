# 0261. Userdata Type + `newproxy` Creation API (N3-C)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

Fourth and final N3 sub-ADR. ADR 0244 pinned userdata as deferred — the Lua spec's standard library has no pure-Lua userdata creation API (it relies on the C FFI `lua_newuserdata`). For LuMeLIR's pure-Lua-no-FFI target, userdata creation needs an explicit Lua-level Builtin to be observable at all.

Lua's de-facto convention is `newproxy(b)` — deprecated officially in 5.0 but still implemented in the reference Lua and commonly emulated. It allocates a userdata; the optional bool flag controls whether the result inherits an empty metatable (`b == true`) or is bare (`b == false`). We adopt it as the creation hook.

This ADR lands:
- A new `TAG_USERDATA` tag in the TaggedValue scheme.
- A new `GC_TYPE_USERDATA` GC type tag.
- The type-system plumbing so `type(ud)` returns `"userdata"`.
- A `Builtin::Newproxy` arm that allocates a GC-tracked userdata payload and wraps it in a TaggedValue slot.

Together with [ADR 0258](0258-close-runtime-dispatch.md), [ADR 0259](0259-gc-finalizer-dispatch.md), and [ADR 0260](0260-mode-weak-clearing.md), this closes the N3 milestone.

## Scope (literal)

- ✅ `TAG_USERDATA = 7` constant (next tag after `TAG_DELETED = 6`).
- ✅ `GC_TYPE_USERDATA = 8` constant + `gc_type_meta` row (`has_outgoing_refs = false`, so the mark-propagation passes skip it).
- ✅ Module global `s_typename_userdata = "userdata\0"`.
- ✅ `emit_type_tagged_local` extended: the innermost else-arm (previously a trap on unknown tag) now checks `tag == TAG_USERDATA` → `"userdata"`, with the trap kept as the truly-unknown fallback.
- ✅ New `Builtin::Newproxy` HIR variant. Lookup table entry `"newproxy"`. Arity `(1, 1)`. Return kind `[ValueKind::TaggedValue]`. Display name `"newproxy"`.
- ✅ Codegen: allocates a 16-byte GC payload via `emit_gc_alloc(GC_TYPE_USERDATA)`. Wraps in a TaggedValue tmp slot — tag `TAG_USERDATA`, payload the allocated ptr.
- ✅ The arg is accepted (HIR-validated by arity 1) but ignored at codegen — the spec `b` flag for empty-metatable inheritance is a future refinement.
- ✅ `type(newproxy(false))` returns `"userdata"`.
- ✅ Userdata values survive `collectgarbage()` when rooted in a chunk-level slot (verified by test 3 — the new `GC_TYPE_USERDATA` is included in the existing chunk root walk via TaggedValue-kind slots).
- ❌ **Userdata equality (`a == b`)** — the existing eq engine traps on `TAG_USERDATA` operand pairs ("table value type mismatch"). Extending the eq dispatch chain with a TAG_USERDATA arm is a separate ADR — uses pointer-identity comparison per Lua spec §3.4.4.
- ❌ **Userdata-specific `__index` / `__newindex` / `__gc` / `__close`** — the existing metamethod dispatches (ADR 0134 / 0135 / 0259 / 0258) trigger off `TAG_TABLE`. Userdata metatables work for value reading but not for metamethod dispatch. Future ADR will lift this — the wiring is a guard relaxation in the existing arms.
- ❌ **Pointer-sized payload customisation** — `newproxy` returns a fixed 16-byte payload. The spec lets userdata carry arbitrary-sized data; future Builtins (`newuserdata(size)` or FFI hooks) will accept a size.
- ❌ **Setmetatable on userdata** — `setmetatable` HIR signature is `[Table, Table]`; accepting userdata at arg 0 requires HIR widening. Out of scope.
- ❌ **`ValueKind::Userdata` enum variant** — adding the variant would ripple through ~50 `match v.kind` sites across HIR + codegen. The TaggedValue-only path lands the same observable behavior without the variant. A future cleanup ADR may introduce the variant if static typing of userdata locals becomes useful (e.g. `local ud: userdata = …` annotations).

## Decision

### Why `newproxy` over `newuserdata`

Lua's spec-supported userdata creation is C-only. `newproxy` is the closest Lua-level emulation — present in the reference Lua source as a deprecated-but-functional built-in for exactly this purpose (creating userdata-typed values from Lua). The name maps to user expectations: anyone reaching for userdata from Lua searches "lua create userdata" and finds `newproxy` references in Lua wiki / SO. Adopting a non-standard name (`__userdata`, `make_userdata`) would diverge from established convention.

### Why TaggedValue-only (no `ValueKind::Userdata`)

`ValueKind::Userdata` would touch every consumer of `ValueKind`: `infer_kind` arms, `param_kinds_for_arity`, `ret_kinds`, codegen alloc + load + store paths, hash-key validity, every `match v.kind` in HIR / codegen. That's ~50 sites and a real surface for bugs. The TaggedValue path lands the observable user behavior (`type(ud) == "userdata"`, GC tracking, round-trip through Locals) without the ripple. If a future ADR introduces statically-typed userdata Locals (`local ud: userdata = …` or similar), it can lift the variant then.

### Why 16-byte payload

The spec lets userdata carry arbitrary data. For `newproxy`'s no-size signature we pick a minimum that:
- Is large enough to participate in GC tracking (the alloc has to be ≥ 1 byte to be unique-identified by `g_gc_head` membership).
- Matches the TaggedValue slot size, making the alloc bookkeeping uniform with other tagged consumers.

Future `newuserdata(size)` Builtin can take an explicit size.

### Why N3 closes here

The roadmap rebuild (2026-06-21) defines N3 as four sub-ADRs (A/B/C/D). With this ADR landing N3-C, the N3 milestone closes. The runtime gaps left behind are:
- N3-B-2 (mark-phase `__mode` skip; the weak-clear pass infra is in place).
- TAG_USERDATA equality + metamethod-dispatch path.
- Real `(Table, …)` arg pass-through for the ADR 0258 / 0259 metamethod ABI.

Each is a focused follow-up rather than blocking the milestone close.

## Tests

`tests/phase4_n3c_userdata.rs` (NEW, 3 e2e, all Green):

1. **`newproxy(false)` returns userdata** — `type(newproxy(false))` prints `"userdata"`.
2. **Two `newproxy` calls each report userdata** — both prints emit `"userdata"`. Equality between userdata is N3-C-2.
3. **Rooted userdata survives collectgarbage** — userdata stored in a chunk slot is rooted via the TaggedValue chunk-root walk; the GC object stays live across `collectgarbage()`.

`tests/phase4_m12_userdata_pin.rs` (the ADR 0244 deferral pin) is superseded by this ADR; its tests stay Green as a documentation-of-prior-state regression net.

## Test count delta

```
Step 0:  1651 (after ADR 0260)
N3-C (impl + 3 new e2e):  1651 → 1654
```

## What this unblocks

- Future C-FFI work can store native pointers in userdata payloads.
- Userdata-specific metamethods (`__index` lookup against the metatable) become a straightforward extension of ADR 0134.
- `newproxy(true)` (auto-attached empty metatable) is a minor codegen extension once the spec-flag inspection lands.

## What still needs follow-up

- N3-B-2: mark-phase `__mode` skip (weak-clear pass infra is ready).
- TAG_USERDATA equality + metamethod dispatch on userdata receivers.
- Metamethod ABI for `(Table, …)` real arg pass-through (umbrella across ADR 0258 / 0259).
- `ValueKind::Userdata` static-typing introduction (only when needed).
- `newuserdata(size)` Builtin for spec-aligned variable-size payloads.

## References

- [ADR 0244](0244-userdata-deferral-pin.md) — predecessor deferral pin; runtime now landed.
- [ADR 0258](0258-close-runtime-dispatch.md) — N3-D `__close` sibling.
- [ADR 0259](0259-gc-finalizer-dispatch.md) — N3-A `__gc` sibling; shared metamethod ABI rationale.
- [ADR 0260](0260-mode-weak-clearing.md) — N3-B `__mode` sibling.
- [ADR 0184](0184-gc-type-meta.md) — `gc_type_meta` table extended.
- [Lua 5.4 §2.1](https://www.lua.org/manual/5.4/manual.html#2.1) — value types including userdata.
- [Lua 5.4 §3.4.4](https://www.lua.org/manual/5.4/manual.html#3.4.4) — equality and userdata identity.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N3-C in the N1-N10 path.
