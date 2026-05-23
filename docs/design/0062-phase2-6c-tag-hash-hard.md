# 0062. Phase 2.6c-tag-hash-hard: Hash Hard Tombstone via Sentinel Key

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0060 (Phase 2.6c-tag-hash) introduced the value-side tagged
slot for hash entries and implemented `t.k = nil` as a **soft
tombstone** — the value tag was set to Nil but the key pointer
was left in place at offset 0 of the entry. The LIC tracker
recorded this as `LIC-2.6c-tag-hash-1`: Lua spec specifies that
`t.k = nil` *physically removes* the key, while we kept it.

ADR 0061 (Phase 2.6c-isnil-query) made the surface-level query
`t.k == nil` answer Lua-compatibly without touching the storage
representation. So observable behaviour was already Lua-correct
end-to-end; what remained was the **structural** mismatch with
the spec, plus two practical drawbacks of the soft form:

- Repeated `t.k = X; t.k = nil` cycles never recover capacity
  until a rehash carrying along all the Nil-tagged entries.
- Probing chains accumulate "ghost" slots that load-factor logic
  cannot see for what they are.

This phase replaces soft tombstones with **hard tombstones**: a
deleted bucket has its key pointer overwritten with the i64
sentinel value `1` (`HASH_DELETED_KEY`). Probing skips past the
sentinel; rehash physically drops sentinel entries. The tag
slot still gets `Nil` written to it on delete, so a subsequent
plain read still finds the entry's value-side `s_table_type_mismatch`
trap (the read can never legitimately reach a sentinel because
the probe skips past).

## Decision

### Sentinel value: `HASH_DELETED_KEY = 1`

```rust
const HASH_DELETED_KEY: i64 = 1;
```

The hash entry's key field is `!llvm.ptr`. Live keys are
addresses of `.data`-section string globals (alignment ≥ 4) or
`malloc`'d buffers (alignment ≥ 8). `null = 0` already marks an
empty bucket. The integer value `1`, viewed as a pointer, can
never collide with any valid key — so it makes a safe sentinel.
Comparison goes through `llvm.ptrtoint` to the existing
`arith.cmpi` path (mirrors the null check).

### Sentinel reuse strategy: **single-pass null-walk, no reuse**

Open-addressing hard tombstones admit a more sophisticated
"first-sentinel reuse" pattern (track the first sentinel
encountered, fall back to it if the probe runs off into a null
slot). This phase deliberately picks the simpler form:

- **Lookup**: walk past sentinels, stop at null (trap or
  missing) or matching key.
- **Insert**: walk past sentinels, stop at null (insert here,
  bump count) or matching key (overwrite). **Do not insert at a
  sentinel.**

Sentinels live until the next rehash. Each rehash physically
drops them, so worst-case wasted capacity is bounded by one
rehash interval. The simplification is one line in each probe
helper instead of an extra mutable register that cuts across the
scf.while shape. Tidy-First gives us the option to add reuse
later if profiling shows it matters.

### Probe helper changes

Both `emit_hash_probe_lookup` and `emit_hash_probe_for_insert`
now compute `is_sentinel = key_at_slot == HASH_DELETED_KEY` and
use it together with `is_null` to gate the strcmp:

```text
is_null     = key == 0
is_sentinel = key == 1
is_skip     = is_null OR is_sentinel
eq          = scf.if is_skip then false else strcmp == 0
```

`emit_hash_probe_lookup` continues to trap on `is_null`. The
`is_sentinel` gating only steers the strcmp around the UB of
calling it on `(void*)1`; the loop continues past the sentinel
for the same reason it continues past a non-matching live key —
`!eq` is true.

`emit_hash_probe_for_insert` uses `found = is_null OR eq` (no
sentinel reuse): a sentinel is treated like a non-match, the
loop walks past it, and the eventual `is_null` becomes the
insertion point.

### IndexAssign Nil arm — hard delete

The hash IndexAssign arm dispatches on `value_kind`:

- **Number**: same as before — when bucket was empty, store the
  key and bump count; then store the tagged number value.
- **Nil** (hard delete): only act when bucket was *not* empty —
  store `HASH_DELETED_KEY` (via `llvm.inttoptr`) at the key
  offset and write Nil tag at the value slot. Empty bucket =
  no-op (deleting an absent key is harmless per Lua spec). count
  stays unchanged either way: the sentinel is still counted
  against load factor until rehash physically drops it.

### Rehash migration filter

The rehash loop in `emit_hash_grow_if_needed` previously gated
migration on `old_key != null`. It now uses
`(old_key != null) AND (old_key != HASH_DELETED_KEY)`. Sentinel
entries are dropped during rehash; only live entries are
re-inserted into the new buffer.

### IsNilQuery (ADR 0061) compatibility

`HirExprKind::IsNilQuery` reuses `emit_hash_probe_for_insert` to
locate a bucket. Under the new probe semantics, the returned
bucket is either null (missing key) or a matching live key —
sentinels are skipped during probing and never appear as the
returned bucket. So IsNilQuery's existing logic (null-key →
true; matching-key → load value-tag and check) stays correct
without modification.

### CA invariants preserved

| Layer    | Change                                                                                          |
|----------|-------------------------------------------------------------------------------------------------|
| Lexer    | None                                                                                            |
| Parser   | None                                                                                            |
| AST      | None                                                                                            |
| HIR      | None (`(String, Nil)` was already accepted in ADR 0060)                                         |
| Codegen  | Sentinel constant; sentinel-skip in both probe helpers; hard-delete in IndexAssign Nil arm; rehash filter |

## TDD Process

1. **Step 1 — Red.** 7 e2e tests in
   `tests/phase2_6c_tag_hash_hard.rs`. Note: because ADR 0061
   already made the *surface* behaviour of delete + isnil-query
   Lua-correct, the soft-tombstone implementation also passes
   these tests — they exist as a regression suite that hard
   tombstones must not break, plus indirect cap-stability
   coverage (`delete_then_reinsert_repeated_no_inflation`,
   `rehash_with_deletes_sums_remaining_evens`).
2. **Step 2 — Green.** Sentinel constant + both probe helpers
   + IndexAssign Nil arm + rehash filter. All 7 hard-tombstone
   tests, all 11 ADR-0060 tests, and all 11 ADR-0061 tests pass
   together at 759 (= 752 + 7).
3. **Step 3 — ADR + AGENTS + commit.** Single feature commit.

## Alternatives Considered

- **First-sentinel reuse on insert.** Tracks the first sentinel
  encountered during probe, falls back to it if the chain ends
  in null. Reduces sentinel pile-up between rehashes at the
  cost of an extra carried register through the scf.while.
  Defer — current rehash cadence is a sufficient cleanup.
- **Decrement count on delete.** Treats the live count as
  "live entries only" so load factor reflects actual fullness;
  but then sentinel chains can grow without triggering a rehash
  and probe walks lengthen unbounded. Rejected — keeping count
  inclusive of sentinels makes rehash the natural cleanup
  trigger.
- **Use `(void*)0xDEAD…` as sentinel.** Larger value, more
  defensive against accidental aliasing, but i64 `1` is already
  unambiguous given alignment constraints, and a small constant
  is one fewer `IntegerAttribute`. Rejected as unneeded.
- **Eager rehash on every delete.** Drops sentinels immediately
  but is O(cap) per delete in the worst case. Rejected.

## Consequences

- ~150 net LOC in `src/codegen/emit.rs` (sentinel constant; both
  probe helpers gain `is_sentinel` + skip gate; IndexAssign Nil
  arm restructured into per-`value_kind` dispatch; rehash gains
  `not_sentinel` AND-gate).
- 7 new e2e tests; total green at 759 (= 752 + 7).
- **LIC-2.6c-tag-hash-1 → resolved**.
- Surface behaviour is unchanged — IsNilQuery / read-after-delete
  trap / write-delete-rewrite cycle / alias semantics all match
  the soft-tombstone era exactly. The win is structural and
  load-factor-related, not user-observable.
- Rehash now physically drops deleted entries; long-running
  delete/insert cycles cannot accumulate ghost capacity.
- The probe helpers are now noticeably similar (both gate
  strcmp on a `key not live` condition). A Tidy-First DRY pass
  may consolidate them later — left out of this commit because
  the gating *predicates* still differ slightly (lookup also
  traps on null; insert does not).

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0061.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `== nil`: true; plain read: trap | partial (ADR 0061) |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number-only | pending tagged values + locals widening |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number-only | pending tagged values |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline `== nil`: true; plain read: trap | partial (ADR 0061) |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number+Nil | partial (ADR 0060) |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | **resolved (this ADR)** |

## Out of Scope

- **Heterogeneous hash values** (Bool/String/Function/Table).
  Pending locals widening so reads can extract them coherently.
- **Locals widening** for `local x = t[i]; if x == nil` —
  separate Phase 2.6c-tag-locals.
- **First-sentinel reuse** on insert — possible follow-up.
- **DRY consolidation of the two probe helpers** — possible
  Tidy-First step once a third user emerges.
- **Iteration** (`pairs(t)` / `ipairs(t)`) — depends on
  heterogeneous reads.
