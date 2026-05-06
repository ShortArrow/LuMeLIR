# 0080. Phase 2.8e-iter-pairs: `for k, v in pairs(t) do ...` Hash Iteration

- **Status:** Accepted
- **Date:** 2026-05-06
- **Deciders:** ShortArrow

## Context

Lua spec §3.3.5 / §3.7.3: `pairs(t)` returns an iterator that
yields every `(k, v)` pair whose value is non-nil, traversing both
the array part and the hash part of the table in **unspecified
order**. Unlike `ipairs(t)` (ADR 0078), `pairs` does *not* stop at
the first nil in the array part — array holes are simply skipped
and the iteration continues into the hash part.

ADR 0078 shipped `ipairs` via parser sugar with HIR-level
desugaring to existing primitives (`While + IndexTagged + IsNil`).
That trick does not apply to `pairs`: the hash-bucket walk is a
new codegen primitive — there is no existing HIR shape that loads
"the i-th occupied hash entry's tagged key/value slots". `pairs`
also has to coexist with the `TAG_DELETED` tombstone path
(ADR 0062 / 0079).

LIC-2.8e-iter-pairs-1 has been the **#1 priority** since ADR 0078.
ADR 0079 (hash key kinds expansion → tagged-key 32B entries with
`TAG_DELETED` tombstone in the key tag word) put the layout in
place; this ADR consumes it.

Codex pre-implementation review (post-ADR-0079, ChatGPT) raised
four concerns; each is addressed in §Decision below:

- **[P1]** Body-driven mutation can free `hash_buf` via
  `emit_hash_grow_if_needed`. A walker that caches `hash_buf` /
  `cap` once at loop entry then dereferences freed memory after a
  rehash. **Mitigation**: per-iteration `header.hash_buf` reload
  with ptr-equality detect; mismatch sets `_broken = true` so the
  next `before`-region cond terminates. Symmetric reload on
  `array_buf` for the array phase.
- **[P2]** Order-fixed test assertions are anti-pattern when Lua
  spec leaves order unspecified. **Mitigation**: a
  `run_lines_sorted` helper for all hash-coverage tests.
- **[P2]** Stmt-level codegen walker becomes throwaway when
  `next()` / generic-for arrives. **Decision**: ADR 0075
  superseder (indirect-call re-enablement) is unscheduled and
  blocks the `next()` ABI; extracting an iterator-step helper now
  is premature. ForPairs is intentionally an opaque HIR shape
  whose codegen will be refactored at the generic-for ADR. See
  §Refactor path.
- **[P2]** Visitation/lookup tests alone don't prove the per-tag
  *key* slot copy is correct. **Mitigation**: a dedicated key-
  materialization test that exercises `type(k)` for all 5 key
  kinds in one table.

## Decision

**Plan A'**: a stmt-level codegen walker. Three-stage shape
(parser sugar → HIR opaque shape → codegen), mirroring ipairs at
the source layer but keeping the lowering opaque so codegen owns
the dual-phase walk.

### Parser

`unwrap_named_unary_call(expr, name)` (a refactor of the previous
`unwrap_ipairs_call`) recognises both `ipairs(table_expr)` and
`pairs(table_expr)` by name. `parse_for` tries `ipairs` first,
falls back to `pairs`, then emits `UnsupportedIterator`
(LIC-2.8e-iter-generic-1) for any other callable. The parser
produces `StmtKind::ForPairs { key_name, val_name, table, body }`,
sibling of `StmtKind::ForIpairs`.

### HIR

`HirStmtKind::ForPairs` is **preserved through to codegen** — no
desugaring at this layer. Field shape:

```rust
ForPairs {
    table_local_id: LocalId, // synthetic Table local
    key_id:         LocalId, // TaggedValue
    val_id:         LocalId, // TaggedValue
    break_id:       LocalId, // Bool — unconditional, never Option
    body:           Vec<HirStmt>,
}
```

`break_id` is unconditionally allocated (unlike `While` / `ForNumeric`
which use `Option<LocalId>`). Both the user `break` and the
codegen-driven rehash-abort path write to it.

The HIR `lower_stmt` arm:
1. Evaluates the table expression in the outer scope.
2. `infer_kind` enforces `Table` — else `TypeMismatch { op:
   "for-in-pairs" }`.
3. Pushes a fresh inner scope and declares the four synthetic
   locals (`__lumelir_iter_t_N`, the user `key_name`, the user
   `val_name`, `_broken_N`).
4. Lowers the user body via `lower_scoped_body_no_push` so each
   user statement gets the canonical `if not _broken then STMT
   end` wrap (ADR 0015 break pattern).
5. Returns `Block { table_init; broken_init; ForPairs { ... } }`.
   No synthetic init for the user `k` / `v` slots — codegen writes
   them at the top of every iteration.

### Codegen `emit_for_pairs`

Two phases sharing the single `_broken` flag:

**Phase 1 — array part walk** (`scf.while %i in 1..=len`):
- Per-iteration: reload `header.array_buf`, ptr-equality check
  against the captured initial pointer; on mismatch (regrow)
  `_broken := true`.
- Compute `slot_ptr = arr_buf_now + (i-1) * 16`.
- Load tag; live = `tag != TAG_NIL` AND `not regrew`.
- Live-then: store `Number(i)` to k slot via
  `emit_value_slot_store_number`; copy 16B from slot_ptr to v slot
  via `emit_copy_value_slot_16b`; run body.
- `i += 1`. Cond: `i <= len AND not _broken`.

**Phase 2 — hash part walk** (`scf.if not_lazy_null AND not _broken`
guards an inner `scf.while %bi in 0..cap_initial`):
- Outer guard reads `header.hash_buf` once and skips phase 2 when
  the lazy buffer was never allocated or `_broken` was set during
  phase 1.
- Per-iteration: reload `header.hash_buf`, ptr-equality with
  captured initial; on mismatch `_broken := true` (rehash detected
  per Codex P1).
- Compute `entry_ptr = hash_buf_now + HASH_OFF_ENTRIES + bi * 32`.
- Load `key_tag`; live = `(key_tag != TAG_NIL) AND (key_tag !=
  TAG_DELETED) AND not rehashed`.
- Live-then: copy 16B from `entry_ptr+0` to k slot, 16B from
  `entry_ptr+16` to v slot, run body.
- `bi += 1`. Cond: `bi < cap_initial AND not _broken`.

The walker reuses the existing `and_not_broken` helper for the
`not _broken` AND-extension and a new local helper
`emit_set_broken_if(cond)` for the rehash-detection write. Both
phases use the new `emit_copy_value_slot_16b` helper from
`tagged.rs` (ADR 0080 commit 1, Tidy First) which is also a
refactor candidate for the existing rehash-migration code at
`emit_hash_grow_if_needed`.

### New codegen helpers (`src/codegen/`)

- `tagged.rs::emit_copy_value_slot_16b(ctx, blk, src_ptr, dst_ptr,
  types, loc)` — 2× i64 raw load/store of a 16-byte tagged slot.
  Tag at +0 and payload at +8 round-trip byte-for-byte regardless
  of the live tag (the same trick the rehash-migration block uses
  inline today).
- `emit.rs::emit_for_pairs(...)` — the top-level walker. Calls
  `emit_for_pairs_array_phase` and `emit_for_pairs_hash_phase`.
- `emit.rs::emit_ptrtoint_i64` — wraps the LLVM `ptrtoint` op for
  the per-iteration ptr-equality check.
- `emit.rs::emit_set_broken_if(cond)` — `if cond then store(true,
  _broken_slot) end`.

### Test surface

`tests/phase2_8e_pairs.rs`, 16 e2e tests. All hash-coverage tests
go through `run_lines_sorted` to avoid pinning Lua-unspecified
visit order. The list:

1. Empty table — body never runs.
2. Array-only — sorted output matches `1..len`.
3. Array with hole (`t[5]=99` after `{1,2}`) — pairs skips holes
   3 and 4.
4. Hash string keys — sorted.
5. Hash bool keys — sorted.
6. Hash function keys — `type(k) == "function"` proof.
7. Hash table keys — `type(k) == "table"` proof.
8. Mixed array + hash — 4 lines sorted, 2 array + 2 hash.
9. Tombstone skip — `t.b = nil` removes b from output.
10. Delete + reinsert — bucket reuse stays observable.
11. Break — `if v == 20 then break end` exits cleanly.
12. Nested pairs — outer × inner, 4 lines.
13. Body writes outer state — `sum = sum + v` aggregation.
14. Key materialization — single table with all 5 key kinds,
    `print(type(k))` produces sorted `["boolean", "function",
    "number", "string", "table"]` (Codex P2 #4 proof).
15. Body writes a separate table — non-iterated mutation works.
16. Body grows the iterated table — terminates without crash via
    P1 abort path (the rehash detection sets `_broken`).

## Alternatives Considered

- **Plan B (HIR-level desugar with new low-level expressions):**
  add `HirExprKind::TableLen`, `HashCap`, `HashEntryKey`,
  `HashEntryValue`, `HashEntryIsLive` and reuse the existing
  `While` shape. **Rejected**: the bucket-index walker is opaque
  to expression-level kind inference (a `HashEntryKey(t, bi)`
  expression has runtime-only kind), so the value-kind matrix in
  `infer_kind` would need a new "any tagged" placeholder and every
  consumer site would gain a runtime-dispatched arm. Net codegen
  LOC similar to Plan A', net HIR complexity strictly higher.
- **Plan C (defer until generic-for):** wait for ADR 0075's
  superseder so `pairs` can be a parser-sugar over `for k, v in
  next, t, nil do ... end`. **Rejected**: `pairs` was the #1
  priority and there is no scheduled work on the indirect-call
  re-enablement.
- **Plan D (memcpy intrinsic for slot copy):** call
  `llvm.memcpy.p0.p0.i64` for the 16B slot copy instead of two
  i64 load/stores. **Rejected**: LLVM constant-folds the small-
  size memcpy back to the same load/store pair, and a libc-call
  noise in unoptimised builds is undesirable. Direct load/store is
  what the rehash migration uses today.

## Refactor Path (when ADR 0075 superseder lands)

`HirStmtKind::ForPairs` is intentionally a leaf-of-the-graph
construct so it can be replaced wholesale rather than gradually
generalised. When the indirect-call re-enablement ships and
`next(t, k)` becomes a callable, ForPairs HIR-desugars to:

```text
for k, v in next, t, nil do
  BODY
end
```

— the canonical Lua spec form. `emit_for_pairs` becomes a deletion
candidate; the dual-phase logic moves into a new `next` builtin
(or an iterator-step helper consumed by both `next` and the
generic-for codegen). The `emit_copy_value_slot_16b` helper
remains useful and doesn't need to change.

## Consequences

- **`LIC-2.8e-iter-pairs-1` → resolved.**
- **New `LIC-2.8e-pairs-tagged-key-write-1` (pending):** `t[k] =
  …` inside a `pairs` body where `k` is the iterator-bound
  TaggedValue local is rejected at HIR by `is_hash_key_eligible`
  (which requires a static, non-`TaggedValue` kind). The natural
  Lua idiom "bump every value" therefore can't be written
  directly today; users must aggregate into a separate table
  (test #15 demonstrates the workaround). Resolution requires
  IndexAssign to grow a TaggedValue-key arm with runtime tag
  dispatch — out of scope for this ADR; tracked as the new LIC
  entry above.
- **Test totals: 920 → 935 green** (16 new e2e in
  `tests/phase2_8e_pairs.rs` + 1 obsolete reject test removed
  from `tests/phase2_8e_ipairs.rs`).
- **LIC totals: 20 / 0 / 5 → 21 / 0 / 4** (resolved /
  partial / pending). One pending added
  (`pairs-tagged-key-write-1`), two pending removed
  (`iter-pairs-1` resolved here).
- **Source LOC**: parser +33, HIR +90 (`HirStmtKind::ForPairs` +
  `lower_stmt` arm + 4 visitor companions), `tagged.rs` +28
  (`emit_copy_value_slot_16b`), `emit.rs` +560 (`emit_for_pairs`
  + the two phase helpers + `emit_ptrtoint_i64` +
  `emit_set_broken_if` + dispatcher arm + `collect_string_pool`
  arm). Tests +295. Doc +290.

## Documentation updates

- [x] §1 slot layout — n/a (tagged-key/value layout already
      documented by ADR 0079).
- [x] §2 producer / source taxonomy — `ForPairs` body-side write
      to user `k` / `v` TaggedValue locals added implicitly via
      the existing TaggedValue producer rules.
- [x] §3 consumer matrix — n/a; consumers are unchanged.
- [x] §4 LIC consolidation — `iter-pairs-1` promoted to Resolved;
      `pairs-tagged-key-write-1` added pending. Totals: **21
      resolved / 0 partial / 4 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Iteration: pairs codegen" subsection
      describing the dual-phase walker and the rehash-detection
      mitigation.
- [x] §7 open questions — `iter-pairs-1` removed; promotion list
      reshuffled (indirect-call re-enablement → #1, full closures
      → #2, etc.).
- [x] §8 ADR index — ADR 0080 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
