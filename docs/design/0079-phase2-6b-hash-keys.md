# 0079. Phase 2.6b-hash-keys: Hash Key Kinds Expansion (Tagged-Key Plan E)

- **Status:** Accepted
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

## Context

Lua spec §3.4.5: any non-`nil`, non-`NaN` value can serve as a
table key. Equality is **raw** — `t[function_a]` and
`t[function_b]` are distinct entries even when the two
functions are observationally identical. LuMeLIR has shipped
`Number` (array part) and `String` keys via ADR 0058 / 0060,
but Bool / Function / Table keys were `TypeMismatch`-rejected
at the HIR layer. `LIC-2.6a-arr-3` tracked this as **partial**
since the original 2.6a-min hash design.

Codex post-ADR-0078 review made this the **#1 priority**:
`pairs(t)` (LIC-2.8e-iter-pairs-1) cannot ship without the key
representation that returns Bool / Function / Table values
through the iteration protocol, and the table-as-value-
container completion that downstream phases (closure-with-
upvalues-in-tables, full closures) build on benefits from a
single, tagged key representation across the runtime.

## Decision

**Plan E — tagged-key 16-byte slot.** A hash entry is now 32
bytes laid out as `{tagged_key_slot[0..16), tagged_value_slot
[16..32)}`. Both halves use the same `{i64 tag, 8-byte
payload}` shape that `array_buf` elements have used since
ADR 0064, so the existing `emit_value_slot_*` family of helpers
(`store_*`, `check_*`) drop in unchanged on the key column.

### New tag and constants

| Constant | Value | Role |
|----------|-------|------|
| `TAG_DELETED` | 6 | Hash tombstone marker; lives in the key tag word at entry+0 when the entry was deleted via `t.k = nil` (ADR 0062 hard tombstone). The probe walks past these; rehash drops them physically. |
| `HASH_ENTRY_SIZE` | 32 (was 24) | Bytes per hash entry. |
| `HASH_ENTRY_OFF_KEY_SLOT` | 0 | Tagged key slot at entry+0..16. |
| `HASH_ENTRY_OFF_VALUE_SLOT` | 16 (was 8) | Tagged value slot at entry+16..32. |

The retired `HASH_DELETED_KEY = 1` ptr sentinel (ADR 0062) is
removed — tombstones now carry `TAG_DELETED` in the key tag
word, eliminating a layer of int↔ptr casts and unifying the
"empty / deleted / live" trichotomy on the key tag.

### Three new tag-dispatched helpers

```rust
// tagged.rs (constants only):
pub(crate) const TAG_DELETED: i64 = 6;

// emit.rs:
fn emit_build_search_key_slot(...)        // alloca + dispatched store
fn emit_hash_key_hash_dispatched(...)     // FNV-1a (String) or i64 × FNV_PRIME (others)
fn emit_hash_key_eq_dispatched(...)       // tag match + per-tag payload compare
```

`emit_hash_key_hash_dispatched` reads the key tag at slot+0;
`TAG_STRING` calls the existing `emit_string_hash` (FNV-1a
over the byte sequence); every other tag loads the payload as
i64 and folds it through the FNV prime once. This matches Lua
spec equality — `t[1.5]` and `t[1.0]` collide only if the
i64 reinterpretation of their f64 bit patterns happens to land
on the same bucket modulo cap, which is fine because the
`eq_dispatched` step then uses `cmpf Oeq` to filter false
positives.

`emit_hash_key_eq_dispatched` returns `i1 false` immediately
when the two slot tags differ (Lua spec: keys of different
kinds are never equal). When the tags match, it dispatches:

| Tag | Payload comparison |
|-----|---------------------|
| `TAG_STRING` | `strcmp(payload_a, payload_b) == 0` |
| `TAG_NUMBER` | `arith.cmpf Oeq` on f64 payloads (NaN ≠ NaN per IEEE-754) |
| `TAG_BOOL` / `TAG_FUNCTION` / `TAG_TABLE` | raw `cmpi Eq` on i64 payload words (pointer-identity for Function/Table per Lua spec) |

### Probe loop refactor

`emit_hash_probe_loop` now takes a `search_key_slot` (16-byte
tagged slot pointer) instead of a bare `key_str: ptr`. Inside:

- Initial bucket index = `emit_hash_key_hash_dispatched(slot) &
  mask`.
- Per-bucket: load tag at entry+0, set `is_skip` = `tag ==
  TAG_NIL || tag == TAG_DELETED`.
- Lookup mode (`trap_on_null=true`): trap on `is_null` before
  the equality dispatch.
- Equality: when `is_skip` is false, `emit_hash_key_eq_
  dispatched(entry_ptr, search_key_slot)`; otherwise yield
  `false` directly (the probe walks past skip buckets without
  doing any payload work).
- Termination: lookup mode terminates only on `eq`; insert
  mode terminates on `is_null || eq`.

### HIR relaxation

```rust
fn is_hash_key_eligible(k: ValueKind) -> bool {
    matches!(k,
        ValueKind::Number | ValueKind::String | ValueKind::Bool
        | ValueKind::Function(_) | ValueKind::Table)
}
```

`Index` and `IndexAssign` route Number keys to the array path
(unchanged) and every other eligible kind to the hash path. Nil
keys remain `TypeMismatch`-rejected (Lua spec disallows `nil`
keys; there is no observable use for them in the array path
either).

The IndexAssign value-kind matrix, previously a per-key-kind
case-by-case switch, simplifies to "any non-Nil value (plus Nil
on a non-Number key as the soft-delete signal)".

### Rehash carries 16-byte tagged keys

`emit_hash_grow_if_needed`'s migrate path used to load the old
key as `ptr` and re-store it via `emit_value_slot_store_string`.
The migrate now copies the 16-byte tagged key slot raw — load
tag (i64) at old_entry+0, store at new_entry+0; load payload
word (i64) at old_entry+8, store at new_entry+8 — preserving
the kind without needing a per-kind store helper.

## Alternatives Considered

- **Plan A (semantically identical to E, named differently)** —
  the survey distinguished a "tagged-key entry layout" plan
  from a "tagged value slot widening" plan but they're the same
  thing. Adopted under the unified Plan E label.
- **Plan B (Number-only first)** — extend Number keys before
  Bool / Function / Table. Rejected: the probe / hash / eq /
  tombstone / rehash code touches every kind dispatch site
  anyway, so a Number-only first cut would be discarded when
  the other kinds land. Codex flagged this as 捨て実装.
- **Plan C (Bool-only minimum)** — same rejection as B at
  smaller scope.
- **Plan D (defer entirely)** — incompatible with Codex
  priority and with `pairs(t)`'s prerequisite structure.
- **Keep `HASH_DELETED_KEY=1` ptr sentinel** — workable on a
  ptr-only key column, but inelegant once the column carries
  tagged keys. The new `TAG_DELETED` is one i64 store at
  delete time and one i64 compare in the probe; keeping the
  sentinel would have required `inttoptr` casts and reading
  the payload word every probe iteration.

## Consequences

- **`LIC-2.6a-arr-3` (partial) → resolved.** Number / String /
  Bool / Function / Table keys all supported.
- **New `LIC-2.6b-hash-key-nil-runtime-1`** (pending): a
  TaggedValue local whose dynamic value is `nil` reaches the
  hash-key codegen via the existing TaggedValue path; today
  the runtime probe trips on `TAG_NIL` and the missing-key
  trap fires, but Lua spec calls for a more specific
  diagnostic ("table index is nil"). Tracked as a runtime
  diagnostic refinement.
- **New `LIC-2.6b-hash-key-nan-runtime-1`** (pending): a
  TaggedValue local whose dynamic value is `NaN` reaches the
  hash-key codegen as a Number key; `emit_hash_key_eq_
  dispatched` uses `cmpf Oeq` (NaN ≠ NaN) so the probe walks
  past the bucket and never finds it again. Lua spec wants a
  hard runtime error at insert time.
- **Test totals: 908 → 920 green.** 12 new e2e in
  `tests/phase2_6b_hash_keys_kinds.rs` (Bool / Function /
  Table identity, distinct-instance, overwrite, delete +
  reinsert, mixed-kind in one table, multi-kind stress
  rehash, nil-key reject backstop) and 2 reframed regression
  tests (the previous `non_arithmetic_key_kind_is_static_
  error_after_2_6b` and `write_key_must_be_arithmetic_or_
  string_after_2_6b` now assert `nil` rejection only).
- **Codegen LOC**: ~+220 net in `emit.rs` (3 new helpers + the
  probe-loop dispatch refactor + 4 IndexAssign / Index /
  IsNil call-site widenings). `tagged.rs` gains a single
  `TAG_DELETED` constant.
- **HIR LOC**: ~+30 (`is_hash_key_eligible` helper + 2 reject-
  site relaxations + the value-kind matrix simplification).

## Documentation updates

- [x] §1 slot layout — hash entry shape table updated to 32
      bytes; `TAG_DELETED = 6` added to the tag space row.
- [x] §2 producer / source taxonomy — `IndexAssign` /
      `Index` rows annotated with the widened key set.
- [x] §3 consumer coverage matrix — n/a (consumer matrix is
      indexed by **value** source, not key; the hash key kinds
      live below the consumer-dispatch surface).
- [x] §4 LIC consolidation — `arr-3` promoted to Resolved;
      `hash-key-nil-runtime-1` and `hash-key-nan-runtime-1`
      added pending. Totals: **20 resolved / 0 partial / 5
      pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Hash key dispatch" subsection
      describing the entry layout and the per-tag hash / eq
      dispatch.
- [x] §7 open questions — `arr-3` removed; `pairs` promoted to
      #1.
- [x] §8 ADR index — ADR 0079 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
