# Tagged Value Semantics

> **Single Source of Truth** for the TaggedValue runtime
> representation introduced across Phase 2.6c (ADRs 0061–0067).
> Update this page whenever a sub-phase changes producer /
> consumer / tag semantics. ADRs continue to record *decisions*;
> this page records *current state*.

**Last updated:** 2026-05-05 (after ADR 0069)

---

## 1. Slot Layout

A "tagged value slot" is **16 bytes**, laid out as
`{i64 tag, 8-byte payload}`:

```text
offset 0  +-------------------+
          |    i64 tag        |   discriminator
offset 8  +-------------------+
          |    8-byte payload |   typed by tag (see §2)
offset 16 +-------------------+
```

Storage sites that use this layout:

- `array_buf` element slots (Phase 2.6c-tag-arr / ADR 0059) —
  `ARRAY_ELEM_SIZE = 16`.
- `hash_buf` entry value slots (Phase 2.6c-tag-hash / ADR 0060) —
  hash entry is `{ptr key, 16-byte tagged value}` totalling 24 B.
- `MaybeNil`-style local alloca (Phase 2.6c-tag-locals / ADR 0063;
  later renamed `TaggedValue` / ADR 0066). Allocated as
  `alloca i64 × 2` for natural 8-byte alignment of the payload.

Constants live in `src/codegen/emit.rs`:

```rust
const ARRAY_ELEM_OFF_VALUE: i64 = 8;   // payload field offset
const TAG_NIL: i64 = 0;
const TAG_NUMBER: i64 = 1;
const TAG_BOOL: i64 = 2;
const TAG_STRING: i64 = 3;
// TAG_FUNCTION = 4 / TAG_TABLE = 5 are reserved for future
// sub-phases (LIC-2.6c-tag-hetero-fn-tbl-1).
```

### Payload type per tag

| Tag         | Payload value type | Notes                                  |
|-------------|--------------------|----------------------------------------|
| TAG_NIL     | `i64 = 0`          | Unused; written as zero for hygiene    |
| TAG_NUMBER  | `f64`              | IEEE-754 double                        |
| TAG_BOOL    | `i64` (zext of i1) | Low bit holds the bool value           |
| TAG_STRING  | `!llvm.ptr`        | Pointer to a `.data`-section global     |
| (reserved)  | —                  | Function/Table tags pending sub-phase  |

Internal slot-to-slot copies load the payload as **raw `i64`**
so any tag round-trips byte-for-byte without a kind-specific
bitcast (ADR 0064).

---

## 2. Producer / Source Taxonomy

A "producer" is any HIR shape (or codegen path) that **writes**
a tagged slot, or whose result **carries** a tagged value.

| Source shape                                | Where it writes / lives                              | Introduced |
|---------------------------------------------|------------------------------------------------------|------------|
| `HirExprKind::Table([elem₀, …])`            | `array_buf` slots, kind-dispatched store             | ADR 0059, 0064 |
| `HirStmtKind::IndexAssign { target, key, value }` (Number key) | `array_buf[key-1]` slot   | ADR 0055, 0059, 0064 |
| `HirStmtKind::IndexAssign { target, key, value }` (String key) | `hash_buf` entry value slot | ADR 0058, 0060, 0064 |
| `HirExprKind::IndexTagged { target, key }`  | LocalInit / Assign **only** — populates a `TaggedValue` slot via `emit_local_init_tagged` | ADR 0063 |
| `HirExprKind::Local(id)` with `info.kind == TaggedValue` | Existing 16-byte alloca holds the tagged value | ADR 0063 |
| Hard-tombstone delete (`t.k = nil`)         | `hash_buf` entry: key→sentinel + value tag→Nil       | ADR 0062 |
| **(future)** function-return widening       | Pending — LIC-2.6c-tag-locals-fn                     | —          |
| **(future)** iterator (`pairs` / `ipairs`)  | Pending — depends on widening                        | —          |
| **(future)** Function / Table tags          | Pending — LIC-2.6c-tag-hetero-fn-tbl-1               | —          |

`HirExprKind::IndexTagged` is **statement-context only**:
calling `emit_expr` on it is `unreachable!()`. It exists purely
to drive `emit_local_init_tagged`.

`infer_kind(IndexTagged) = TaggedValue` (HIR side); the
underlying `HirExprKind::Index` still infers `Number` for
backward compatibility (ADR 0063 design choice — preserve the
trapping-Index path for sites the widening rewrite does not
touch).

---

## 3. Consumer Coverage Matrix

A "consumer" is any HIR / codegen site that **reads** a tagged
value (or accepts one as an operand). The cells describe the
runtime behaviour for each tag.

Legend:
- `%g` — `printf`/`snprintf` `%.14g` (IEEE-754 formatting)
- `s_*` — pointer to a `.data` global string
- "trap" — `s_table_type_mismatch` exit(1) (Lua spec for
  arith/cmp on incompatible kinds)

### `print(x)`

| Source                              | Number  | Bool      | String  | Nil    | ADR  |
|-------------------------------------|---------|-----------|---------|--------|------|
| inline `Index { … }`                | `%g`    | s_true/false | `%s` | s_nil  | 0065 |
| `Local(TaggedValue)`                | `%g`    | s_true/false | `%s` | s_nil  | 0064 |
| `IndexTagged` (statement-only)      | n/a — never reaches expression context              ||| 0063 |

Implementation path: `Builtin::Print` arg loop special-cases
both shapes; inline `Index` materialises through a tmp tagged
slot via `emit_local_init_tagged` + `emit_print_tagged_local`.

### `type(x)`

| Source                              | Number      | Bool        | String      | Nil       | ADR  |
|-------------------------------------|-------------|-------------|-------------|-----------|------|
| `Local(TaggedValue)`                | `"number"`  | `"boolean"` | `"string"`  | `"nil"`   | 0067 |
| inline `Index`                      | `"number"`* | `"number"`* | `"number"`* | `"number"`* | LIC-2.6c-tag-consumers-inline-1 |

`*` — static dispatch returns `"number"` regardless of runtime
tag. Open LIC.

### `tostring(x)`

| Source                              | Number    | Bool          | String         | Nil      | ADR  |
|-------------------------------------|-----------|---------------|----------------|----------|------|
| `Local(TaggedValue)`                | `%g` snprintf | `s_true`/`s_false` | payload ptr | `s_nil` | 0067 |
| inline `Index`                      | trap (extract f64 path)                                       |||| LIC-2.6c-tag-consumers-inline-1 |

`..` (concat) auto-coerces non-String operands via
`tostring(...)` (ADR 0026), so concat with a `Local(TaggedValue)`
inherits the runtime dispatch for free (matrix tests cover
this).

**Reserved tags (TAG_FUNCTION = 4 / TAG_TABLE = 5)**: every
runtime-dispatch consumer (`print`, `type`, `tostring`,
Local-Local `==`) traps via `emit_tagged_unknown_tag_trap` (ADR
0069) when an unsupported tag reaches the dispatch chain.
Currently unreachable — HIR rejects Function / Table values in
tables (LIC-2.6c-tag-hetero-fn-tbl-1) — but the trap is the
fail-fast guard rail for the day a sub-phase begins lowering
those tags.

### `==` / `~=` (tagged operand)

| Source LHS                          | Source RHS              | Behaviour                                | ADR  |
|-------------------------------------|-------------------------|------------------------------------------|------|
| inline `Index`                      | `Nil` literal           | non-trapping `IsNil(Index{…})`            | 0061 |
| `Local(TaggedValue)`                | `Nil` literal           | non-trapping `IsNil(Local(…))`            | 0063 |
| `Local(TaggedValue)`                | Number / Bool / String literal | tag check + per-kind compare        | 0065 |
| `Local(TaggedValue)`                | `Local(TaggedValue)`    | tag-vs-tag dispatch + per-kind compare; both Nil → true | 0066 |

`Ne` is `UnaryOp::Not(Eq)` throughout (HIR rewrite). The
`HirExprKind::IsNil(Box<HirExpr>)` variant unifies the Index
and Local source shapes (ADR 0066, formerly two variants).

### Arith / ordering on tagged operand

| Operator                            | TAG_NUMBER       | TAG_BOOL  | TAG_STRING  | TAG_NIL    | Lua spec             |
|-------------------------------------|------------------|-----------|-------------|------------|----------------------|
| `+ - * / % ^ // & \| ~ << >>`       | extract f64; arith | trap     | trap        | trap       | `nil + 1` errors     |
| `< <= > >=`                         | extract f64; cmpf | trap     | trap        | trap       | mixed kinds error    |

These traps are **Lua-spec correct** for the current tag set —
no LIC entry. (`"5" + 1` coercion is a separate Lua-spec
feature; see Open Questions.)

---

## 4. LIC Status (consolidated)

LIC entries from ADRs 0054 / 0058 / 0061-0067. Status is the
**latest** state; the ADR column points to the resolution (or
the introduction, when still open).

### Resolved

| ID                                | Resolution                                  | ADR(s)        |
|-----------------------------------|---------------------------------------------|---------------|
| LIC-2.6a-arr-1                    | OOB array read returns nil at all surfaces  | 0061+0063+0065 |
| LIC-2.6a-wr-1                     | hole write creates Nil-tagged slot          | 0059          |
| LIC-2.6a-wr-2                     | grow write extends length                   | 0057          |
| LIC-2.6b-hash-1                   | missing hash key returns nil at all surfaces | 0061+0063+0065 |
| LIC-2.6c-tag-hash-1               | `t.k = nil` sentinel + rehash drop          | 0062          |
| LIC-2.6c-tag-locals-1             | `type(x)` runtime dispatch                  | 0067          |
| LIC-2.6c-tag-hetero-eq-1          | `==`/`~=` Local-Local runtime dispatch      | 0066          |
| LIC-2.6c-tag-hetero-inline-1      | inline `print(t[k])` runtime dispatch       | 0065          |

### Partial

| ID                                | Resolved range                              | Pending range                    |
|-----------------------------------|---------------------------------------------|----------------------------------|
| LIC-2.6a-arr-2                    | Bool/String values via tagged slot (ADR 0064) | Function/Table — see fn-tbl-1   |
| LIC-2.6a-wr-3                     | Bool/String writes (ADR 0064)               | Function/Table — see fn-tbl-1    |
| LIC-2.6b-hash-2                   | Bool/String hash values + Nil-delete (ADR 0064) | Function/Table — see fn-tbl-1 |
| LIC-2.6a-arr-3                    | Number + String keys (ADR 0058)             | Bool/Function/Table keys         |

### Pending

| ID                                | Behaviour                                                             | Notes                          |
|-----------------------------------|-----------------------------------------------------------------------|--------------------------------|
| LIC-2.6c-tag-hetero-fn-tbl-1      | Function/Table values rejected by HIR                                 | Needs ucast / cycle / closure-escape work |
| LIC-2.6c-tag-consumers-inline-1   | inline `type(t[k])` / `tostring(t[k])` static-dispatch                | Mirror of ADR 0067 for inline form |

**Total:** 12 LIC entries — 8 resolved, 3 partial, 2 pending
(2 partial entries roll into `fn-tbl-1`; the partial form is
preserved for granular tracking).

---

## 5. Runtime Tag Invariants

Hold across all sub-phases unless an ADR explicitly amends them.

1. **Tag at offset 0**, payload at offset 8. Always `i64` tag.
2. **Tag identity** determines payload type. Mis-typed payload
   read is undefined; the consumer must dispatch on tag before
   bit-casting.
3. **Tag mutation** is statement-scoped. Only `LocalInit` /
   `Assign` / `IndexAssign` rewrite a tagged slot's tag. No
   mid-expression mutation.
4. **Hash sentinel** (`HASH_DELETED_KEY = 1`, ADR 0062) lives in
   the **key** field of a hash entry, never in the payload of
   a tagged slot. Tagged-slot payload semantics are independent
   of the hash sentinel layer.
5. **Defensive `else:` fallback** in any `scf.if` chain over
   tags **traps** via `emit_tagged_unknown_tag_trap` (ADR 0069)
   when the runtime tag is reserved (Function = 4 / Table = 5)
   and the consumer has no implementation for it. Yielding a
   silent value (e.g. `s_typename_function`, `false`, `s_nil`)
   is a bug — it would mis-identify a future tag value as a
   currently-supported one. The trap reuses
   `s_table_type_mismatch` so the diagnostic is consistent with
   the array/hash trap surface (ADR 0059 / 0060). Backed by
   `tests/phase2_6c_tag_defensive_trap.rs` — HIR rejects
   Function / Table values into tables today, so the trap is
   currently unreachable; it stays as a fail-fast guard rail
   for the day a sub-phase starts populating reserved tags.
6. **Slot copy** between two tagged slots reads the payload as
   raw `i64` (ADR 0064) so any tag value round-trips.

---

## 6. Producer-Consumer Cross-Reference (How To)

When **adding a new consumer** of TaggedValue (e.g. a builtin
that consumes a Local value):

1. Identify the operand's static `ValueKind`. If it can be
   `TaggedValue`, the consumer must dispatch.
2. In the consumer's codegen arm, special-case the
   `(HirExprKind::Local(_), TaggedValue)` shape **before**
   calling `emit_expr` (which extracts as f64 and traps).
3. Read the tag at offset 0 with `emit_load(slot_ptr, types.i64)`.
4. Build a `scf.if` chain (or a switch) that dispatches on tag.
   Reuse the layout helpers in `emit.rs`:
   - `emit_byte_offset_ptr(slot, ARRAY_ELEM_OFF_VALUE)` →
     payload pointer
   - `emit_load(payload_ptr, payload_type)` per tag
5. Add a row to the consumer matrix in this document.
6. Add cells to `tests/phase2_6c_tag_consumers_matrix.rs` for
   each `(consumer × runtime tag)`.
7. If the consumer's pre-existing static-kind path needs to
   stay as a fallback (e.g. for non-Local operands), keep it
   with a comment pointing here.

When **adding a new producer** (e.g. function-return widening):

1. Decide whether the producer fits an existing slot site
   (`array_buf`, `hash_buf`, alloca) or needs a new one.
2. Choose the HIR shape: a new `HirExprKind` variant, an
   existing `Local` whose kind becomes `TaggedValue`, or a
   wrapper expression similar to `IndexTagged`.
3. Update §2 with the new source shape.
4. Verify all consumer rows in §3 cover the new source.
5. Add ADR with the design decision; update this doc.

---

## 7. Open Questions / Known Gaps

Listed in Codex review priority order (post-ADR-0067):

1. **`tagged.rs` module split (Tidy First).** `emit.rs` is
   ~7800 LOC; tagged-related helpers (constants, store
   helpers, dispatch helpers) total ~1900 LOC. A clean split
   needs careful visibility / re-export work; deferred to a
   dedicated phase. ADR 0067 explicitly defers.
2. **Function-return TaggedValue widening.** `local x = f()`
   where `f` returns nil/heterogeneous should widen `x`.
   Requires function ABI updates (return a 16-byte tagged
   payload, or pass a pointer). Most natural follow-up to the
   matrix scaffold (extends the source axis).
3. **Inline `type(t[k])` / `tostring(t[k])`** (LIC-2.6c-tag-
   consumers-inline-1). Mirror of ADR 0067 for inline `Index`
   sources.
4. **Function/Table values in tables** (LIC-2.6c-tag-hetero-
   fn-tbl-1). Needs ucast / cycle / closure-escape work for
   the payload representations.
5. **String → Number coercion on arith** (`"5" + 1`). Lua-spec
   feature, mostly orthogonal to the tagged-value redesign.
6. **Iteration `pairs(t)` / `ipairs(t)`.** Depends on the
   widened source set.
7. **Hash key kinds expansion** (LIC-2.6a-arr-3). Bool /
   Function / Table keys.
8. **Full closures** (`2.5c-full`). Independent track; heap-
   allocated environments.

---

## 8. ADR Index (chronological)

| ADR  | Phase                        | Contribution                                                        |
|------|------------------------------|---------------------------------------------------------------------|
| 0054 | 2.6a-arr                     | Array constructor + integer indexing read                          |
| 0055 | 2.6a-wr                      | Number-array element write                                         |
| 0057 | 2.6a-grow                    | Array push (`t[#t+1] = v`)                                         |
| 0058 | 2.6b-hash                    | String-keyed hash; FNV-1a + linear probing                         |
| 0059 | 2.6c-tag-arr                 | 16-byte tagged array slot; hole write fix                          |
| 0060 | 2.6c-tag-hash                | 24-byte hash entry; soft `t.k = nil`                               |
| 0061 | 2.6c-isnil-query             | Inline `t[k] == nil` non-trapping query                            |
| 0062 | 2.6c-tag-hash-hard           | Hard-tombstone hash delete                                         |
| 0063 | 2.6c-tag-locals              | `local x = t[k]` widens to `MaybeNilNumber` (later TaggedValue)    |
| 0064 | 2.6c-tag-hetero              | Bool / String table values; runtime print dispatch                 |
| 0065 | 2.6c-tag-hetero-fix          | Inline print + Local-Literal Eq runtime dispatch                   |
| 0066 | 2.6c-tag-hetero-eq           | IsNil unification (Tidy First) + Local-Local Eq runtime dispatch   |
| 0067 | 2.6c-tag-consumers           | `type` / `tostring` runtime dispatch + consumer matrix scaffold    |
| 0068 | 2.6c-tag-doc-consolidate     | This SoT doc + LIC consolidation                                   |
| 0069 | 2.6c-tag-defensive-trap      | Trap on unknown tagged-slot tag (replaces silent fallbacks)        |
