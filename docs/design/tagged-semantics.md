# Tagged Value Semantics

> **Single Source of Truth** for the TaggedValue runtime
> representation introduced across Phase 2.6c (ADRs 0061–0067).
> Update this page whenever a sub-phase changes producer /
> consumer / tag semantics. ADRs continue to record *decisions*;
> this page records *current state*.

**Last updated:** 2026-05-06 (after ADR 0079)

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
- `hash_buf` entries (Phase 2.6c-tag-hash / ADR 0060, widened
  by ADR 0079) — each entry is `{16-byte tagged key, 16-byte
  tagged value}` totalling 32 B. Both halves share the array
  element layout so `emit_value_slot_*` helpers work on each.
  Empty buckets carry `TAG_NIL` in the key tag; deleted buckets
  carry `TAG_DELETED`.
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
// Phase 2.6c-tag-fn-tbl (ADR 0071): closure-less Function and
// Table values now use these tags. Closures with upvalues are
// HIR-rejected (LIC-2.6c-tag-hetero-closure-escape-1).
const TAG_FUNCTION: i64 = 4;
const TAG_TABLE: i64 = 5;
// Phase 2.6b-hash-keys (ADR 0079): hash-bucket tombstone tag.
// Lives in the key tag word at entry+0 when the entry was
// deleted via `t.k = nil`. Probe walks past these; rehash
// drops them physically.
const TAG_DELETED: i64 = 6;
```

### Payload type per tag

| Tag         | Payload value type | Notes                                  |
|-------------|--------------------|----------------------------------------|
| TAG_NIL     | `i64 = 0`          | Unused; written as zero for hygiene    |
| TAG_NUMBER  | `f64`              | IEEE-754 double                        |
| TAG_BOOL    | `i64` (zext of i1) | Low bit holds the bool value           |
| TAG_STRING  | `!llvm.ptr`        | Pointer to a `.data`-section global     |
| TAG_FUNCTION| `!llvm.ptr`        | Function pointer via `unrealized_cast` (ADR 0019); ADR 0071 |
| TAG_TABLE   | `!llvm.ptr`        | Stable table header pointer (ADR 0056); ADR 0071 |
| TAG_DELETED | (unused)           | Hash tombstone marker — only ever appears in a hash entry's **key** tag word; payload is left undefined; ADR 0079 |

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
| `HirStmtKind::IndexAssign { target, key, value }` (Number key) | `array_buf[key-1]` slot — value can be Number / Bool / String / Function (closure-less) / Table | ADR 0055, 0059, 0064, 0071 |
| `HirStmtKind::IndexAssign { target, key, value }` (non-Number key) | `hash_buf` entry — key occupies the 16-byte tagged key slot at entry+0 (Number / String / Bool / Function / Table; nil rejected), value at entry+16 (any non-Nil kind, plus Nil for soft-delete) | ADR 0058, 0060, 0064, 0071, 0079 |
| `HirExprKind::Table([elem, …])`             | `array_buf` slot per elem — same kind set as IndexAssign | ADR 0064, 0071 |
| `HirExprKind::IndexTagged { target, key }`  | LocalInit / Assign **only** — populates a `TaggedValue` slot via `emit_local_init_tagged` | ADR 0063 |
| `HirExprKind::Local(id)` with `info.kind == TaggedValue` | Existing 16-byte alloca holds the tagged value | ADR 0063 |
| Hard-tombstone delete (`t.k = nil`)         | `hash_buf` entry: key tag → `TAG_DELETED`, value tag → Nil (ADR 0079 retired the prior `HASH_DELETED_KEY=1` ptr sentinel) | ADR 0062, 0079 |
| Function-return widening (`Callee::User`)   | `_ret_value_N` slot widens to TaggedValue when same return position sees mixed kinds; ABI returns 2 MLIR results `(i64 tag, i64 payload_raw)` per TaggedValue position | ADR 0074 |
| **(future)** iterator (`pairs` / `ipairs`)  | Pending — depends on widening                        | —          |
| **(future)** closure with upvalues          | HIR-rejects today — LIC-2.6c-tag-hetero-closure-escape-1 | —      |

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

| Source                              | Number  | Bool      | String  | Nil    | Function | Table | ADR  |
|-------------------------------------|---------|-----------|---------|--------|----------|-------|------|
| inline `Index { … }`                | `%g`    | s_true/false | `%s` | s_nil  | `s_typename_function` | `s_typename_table` | 0065 + 0071 |
| `Local(TaggedValue)`                | `%g`    | s_true/false | `%s` | s_nil  | `s_typename_function` | `s_typename_table` | 0064 + 0071 |
| inline `Call(User)` returning TaggedValue | `%g` | s_true/false | `%s` | s_nil  | `s_typename_function` | `s_typename_table` | 0074 |
| `IndexTagged` (statement-only)      | n/a — never reaches expression context                                  |||||| 0063 |

Implementation path: `Builtin::Print` arg loop special-cases
both shapes; inline `Index` materialises through a tmp tagged
slot via `emit_local_init_tagged` + `emit_print_tagged_local`.

### `type(x)`

| Source                              | Number      | Bool        | String      | Nil       | Function       | Table       | ADR  |
|-------------------------------------|-------------|-------------|-------------|-----------|----------------|-------------|------|
| `Local(TaggedValue)`                | `"number"`  | `"boolean"` | `"string"`  | `"nil"`   | `"function"`   | `"table"`   | 0067 + 0071 |
| inline `Index`                      | `"number"`  | `"boolean"` | `"string"`  | `"nil"`   | `"function"`   | `"table"`   | 0070 + 0071 |
| inline `Call(User)` returning TaggedValue | `"number"` | `"boolean"` | `"string"` | `"nil"` | `"function"` | `"table"` | 0074 |

### `tostring(x)`

| Source                              | Number    | Bool          | String         | Nil      | Function     | Table     | ADR  |
|-------------------------------------|-----------|---------------|----------------|----------|--------------|-----------|------|
| `Local(TaggedValue)`                | `%g` snprintf | `s_true`/`s_false` | payload ptr | `s_nil` | `"function"` | `"table"` | 0067 + 0071 |
| inline `Index`                      | `%g` snprintf | `s_true`/`s_false` | payload ptr | `s_nil` | `"function"` | `"table"` | 0070 + 0071 |
| inline `Call(User)` returning TaggedValue | `%g` snprintf | `s_true`/`s_false` | payload ptr | `s_nil` | `"function"` | `"table"` | 0074 |

Both inline rows use the ADR 0065 print pattern, factored into
`emit_inline_index_into_tagged_tmp` (Tidy First, ADR 0071):
`Builtin::Type` / `Builtin::ToString` allocate a tmp 16-byte
slot, fill it via `emit_local_init_tagged` (non-trapping read),
then dispatch via `emit_type_tagged_local` /
`emit_tostring_tagged_local`. Function / Table emit the literal
typename string (`"function"` / `"table"`); address-prefixed
forms (`"function: 0x..."`) are out of scope.

`..` (concat) auto-coerces non-String operands via
`tostring(...)` (ADR 0026), so concat with a `Local(TaggedValue)`
or inline `Index` inherits the runtime dispatch for free
(matrix tests cover both shapes).

**Truly-unknown tag (≥ 6)**: every runtime-dispatch consumer
(`print`, `type`, `tostring`, Local-Local `==`) still traps via
`emit_tagged_unknown_tag_trap` (ADR 0069) for tag values that
neither the supported set (Number/Bool/String/Nil/Function/Table)
nor a future sub-phase has wired up. Today the path is
unreachable — the HIR `value_ok` matrix only emits tags 0–5.

### `==` / `~=` (tagged operand)

| Source LHS                          | Source RHS              | Behaviour                                | ADR  |
|-------------------------------------|-------------------------|------------------------------------------|------|
| inline `Index`                      | `Nil` literal           | non-trapping `IsNil(Index{…})`            | 0061 |
| `Local(TaggedValue)`                | `Nil` literal           | non-trapping `IsNil(Local(…))`            | 0063 |
| `Local(TaggedValue)`                | Number / Bool / String literal | tag check + per-kind compare        | 0065 |
| `Local(TaggedValue)`                | `Local(TaggedValue)`    | tag-vs-tag dispatch + per-kind compare; both Nil → true; Function / Table → ptr equality (Lua reference equality) | 0066 + 0071 |

`Ne` is `UnaryOp::Not(Eq)` throughout (HIR rewrite). The
`HirExprKind::IsNil(Box<HirExpr>)` variant unifies the Index
and Local source shapes (ADR 0066, formerly two variants).

### `f(...)` — calling a TaggedValue callee

| Source                                | All tags                                                                                                               | ADR        |
|---------------------------------------|------------------------------------------------------------------------------------------------------------------------|------------|
| `Local(TaggedValue)` as call callee   | **Rejected at HIR** (`HirError::IndirectCallThroughTaggedLocal`). ADR 0072 reconstructed `(f64,…) → f64` from `args.len()` but that path was UB on arity / return-ABI mismatch; ADR 0075 removes it. Workaround: bind via a known FuncId path or expand a static dispatch at the call site. | 0072 / 0075 |

`Callee::Indirect` is now reserved for `Function(arity)` locals
(parameters with body-scan-inferred arity, or aliases of a
top-level / `local function` definition with a known
`FuncId`). TaggedValue-kind locals — typically bound from a
table read — never reach the indirect call site after this
phase.

### Arith / ordering on tagged operand

| Operator                            | TAG_NUMBER       | TAG_BOOL  | TAG_STRING  | TAG_NIL    | Lua spec             |
|-------------------------------------|------------------|-----------|-------------|------------|----------------------|
| `+ - * / % ^ // & \| ~ << >>`       | extract f64; arith | trap     | trap        | trap       | `nil + 1` errors     |
| `< <= > >=`                         | extract f64; cmpf | trap     | trap        | trap       | mixed kinds error    |

These traps are **Lua-spec correct** for the current tag set —
no LIC entry.

**String operand coercion (ADR 0077):** when a static-`String`
expression appears as an arithmetic / bitwise BinOp operand,
HIR wraps it in `HirExprKind::ArithStringCoerce` and codegen
runs `sscanf("%lf")` at runtime. Successful parse → arith
proceeds; failed parse → exit with
`s_arith_coerce_failed` (Lua-spec runtime error). Distinct
from the `Builtin::ToNumber` builtin path (ADR 0028) whose
failure returns the NaN sentinel — the arith path needs the
trap because Lua spec §3.4.1 disallows silent NaN
propagation from a non-numeric string.

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
| LIC-2.6c-tag-consumers-inline-1   | inline `type(t[k])` / `tostring(t[k])` runtime dispatch | 0070 |
| LIC-2.6c-tag-hetero-fn-tbl-1      | Function (closure-less) and Table values storable      | 0071          |
| LIC-2.6a-arr-2                    | All six tag kinds supported as table elements           | 0064 + 0071   |
| LIC-2.6a-wr-3                     | All six tag kinds supported as IndexAssign values       | 0064 + 0071   |
| LIC-2.6b-hash-2                   | All six tag kinds supported as hash values + Nil-delete | 0064 + 0071   |
| LIC-2.6c-tag-hetero-fn-tbl-call-1 | Calling a Function value retrieved through a tagged slot — resolved by removal in ADR 0075 (Strict Plan C) | 0072 / 0075 |
| LIC-2.6c-tag-locals-fn-1          | Heterogeneous direct-call return widening (`Callee::User`) | 0074        |
| LIC-2.6c-tag-callee-arity-1       | Tagged-callee arity / signature reconstruction soundness — resolved by HIR-rejecting all TaggedValue indirect calls | 0075       |
| LIC-2.6c-tag-locals-fn-indirect-1 | Calling a TaggedValue-returning function through `Callee::Indirect` — subsumed by ADR 0075's broader rejection | 0074 / 0075 |
| LIC-2.6c-tag-locals-fn-multi-1    | Multi-position TaggedValue interleaving (`return 1, nil` vs `return nil, 1`) — caller-side result-index walker generalised | 0076       |
| LIC-2.7p-arith-coerce-1           | String → Number arithmetic coercion (`"5" + 1`); failure traps via `s_arith_coerce_failed` | 0077      |
| LIC-2.8e-iter-ipairs-1            | `for i, v in ipairs(t) do … end` parser sugar with first-nil termination | 0078      |
| LIC-2.6a-arr-3                    | All hash key kinds (Number / String / Bool / Function / Table) via tagged-key 32-byte entry layout | 0058 / 0079 |

### Partial

(none)

### Pending

| ID                                          | Behaviour                                                             | Notes                          |
|---------------------------------------------|-----------------------------------------------------------------------|--------------------------------|
| LIC-2.6c-tag-hetero-closure-escape-1        | Closure with upvalues stored in tables                                | HIR-rejects today (ADR 0044 + ADR 0071); needs escape-analysis relaxation |
| LIC-2.7p-arith-coerce-tagged-1              | TaggedValue operand arith coerce (`local x = t[1]; print(x + 1)` when x is runtime String) | HIR can't statically resolve the kind; current TaggedValue-arith path traps on non-Number tag (ADR 0063). Unlocking needs runtime tag dispatch in arith codegen |
| LIC-2.8e-iter-pairs-1                       | `pairs(t)` hash-bucket iteration                                      | Requires hash-walk protocol design with tombstone awareness (ADR 0062). The tagged-key layout (ADR 0079) is the prerequisite layout for emitting key/value pairs through the iterator |
| LIC-2.8e-iter-generic-1                     | Generic-for protocol with arbitrary callable iterator (`for x in iter, state, init do …`) | Requires reopening the ADR 0075 indirect-call reject via signature side table or equivalent runtime descriptor |
| LIC-2.6b-hash-key-nil-runtime-1             | Dynamic `nil` hash key via TaggedValue local — runtime probe currently fires the generic missing-key trap; Lua spec wants a specific "table index is nil" diagnostic | ADR 0079 |
| LIC-2.6b-hash-key-nan-runtime-1             | Dynamic `NaN` hash key via TaggedValue local — `cmpf Oeq` excludes NaN (NaN ≠ NaN), so the probe walks past and never finds the bucket. Lua spec wants a hard runtime error at insert time | ADR 0079 |

**Total:** 23 LIC entries — 20 resolved, 0 partial, 3 pending core + 2 pending runtime-diag.

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

### Module layout (ADR 0073)

`src/codegen/` is split three ways. Use the table to decide
where a new helper belongs:

| Module          | Responsibility                                                 |
|-----------------|----------------------------------------------------------------|
| `primitive.rs`  | Pure MLIR / LLVM-dialect wrappers. No Lua semantics. `Types`, libc-call shells (`emit_libc_call_*`), `emit_load`/`emit_store`/`emit_byte_offset_ptr*`, `emit_unrealized_cast`, `emit_addressof`, `emit_printf`, `emit_exit_with_message`. Used by both `emit.rs` and `tagged.rs`. |
| `tagged.rs`     | TaggedValue runtime model. Tag space + slot-layout constants, `emit_alloca_slot_for_kind`, all `emit_value_slot_store_*` / `_check_*` helpers, `emit_tagged_unknown_tag_trap`, and the pure-tag consumer dispatchers (`emit_print_tagged_local`, `emit_tagged_eq_local_local`, `emit_type_tagged_local`). Depends only on `primitive.rs` and `crate::hir::ValueKind`. |
| `emit.rs`       | HIR lowering driver, table/hash/array/string codegen, builtin dispatch, control flow, function emission, plus the **statement-context** tagged materializers (`emit_local_init_tagged`, `emit_inline_index_into_tagged_tmp`, `emit_isnil_index`, `emit_tagged_eq_runtime_dispatch`, `emit_tostring_tagged_local`) that recurse through `emit_expr` / call `emit_tostring`. |

A new helper that takes a `slot_ptr` and dispatches purely off
the runtime tag belongs to `tagged.rs`. A helper that recurses
through `emit_expr` (lowers a `HirExpr`) belongs to `emit.rs`.
A helper that wraps a single MLIR / LLVM op without touching
Lua semantics belongs to `primitive.rs`.

### Indirect call safety boundary (ADR 0075)

`Callee::Indirect` is the codegen path for calling a function
value through a local. Its acceptance is now bounded by what
HIR can statically prove about the callee's arity:

| Local kind                              | Arity source                            | Indirect call accepted? |
|-----------------------------------------|-----------------------------------------|-------------------------|
| `Function(arity)` parameter             | inferred via body scan (ADR 0018)       | ✅ (arity validated upfront in `lower_call`) |
| `Function(arity)` alias of named fn     | `info.func_id` resolves to a `FuncId`   | ✅ (validated; `Callee::User` shortcut for the common case) |
| `Function(arity)` from non-Index source | static ABI from the binding expression  | ✅ (validated) |
| `TaggedValue` from any source           | (no static descriptor)                  | ❌ HIR rejects (`HirError::IndirectCallThroughTaggedLocal`, ADR 0075) |

ADR 0072 had previously enabled the TaggedValue case by
reconstructing the function type from `args.len()` at codegen
time, but the reconstruction was unsound when the source
table held heterogeneous-arity or heterogeneous-return-ABI
functions. ADR 0075 closes the UB root cause by rejecting
the unsafe path; the supported workaround is to bind the
callable through one of the static-arity paths above, or to
expand the dispatch as named-function calls inside a wrapper.

A future phase (signature side table, or full closures) may
re-enable the table-derived case without the soundness gap.

### Function-return ABI (ADR 0074)

A `HirFunction::ret_kinds[i] = ValueKind::TaggedValue` lowers
to **two** MLIR results at position `i`:

```text
func.func @user_fn_NN(...) -> (i64, i64)
                              ^^^   ^^^
                              tag   payload_raw
```

`payload_raw` is **i64**, the universal representation used
by slot-to-slot copies (ADR 0064). The caller reads
`op_ref.result(0)` as the tag and `op_ref.result(1)` as the
raw payload, then stores both into a 16-byte tagged slot via
direct `emit_store` calls (no kind dispatch needed —
the data is already in slot-compatible form). Helpers
[`emit_call_user_into_tagged_slot`] and
[`emit_call_user_into_tagged_tmp`] in `emit.rs` encapsulate
this pattern for the LocalInit/Assign destination and the
inline-consumer (Print / Type / ToString) tmp-slot
materialization respectively.

**Multi-position interleaving (ADR 0076):** any combination of
return-position kinds is supported, including multiple
TaggedValue positions. The caller-side result-index walker
(`flat_result_index` + `emit_pack_tagged_result_at_pos`) maps
each logical position to its non-overlapping MLIR result range:

| `ret_kinds`                              | MLIR signature                          | Position → result indices |
|------------------------------------------|-----------------------------------------|---------------------------|
| `[Number]`                               | `() → f64`                              | pos 0 → result 0          |
| `[TaggedValue]`                          | `() → (i64, i64)`                       | pos 0 → results 0..2      |
| `[Number, TaggedValue]`                  | `() → (f64, i64, i64)`                  | pos 0 → 0; pos 1 → 1..3   |
| `[TaggedValue, TaggedValue]`             | `() → (i64, i64, i64, i64)`             | pos 0 → 0..2; pos 1 → 2..4 |
| `[Number, TaggedValue, Bool]`            | `() → (f64, i64, i64, i1)`              | pos 0 → 0; pos 1 → 1..3; pos 2 → 3 |

Each TaggedValue position contributes 2 MLIR results
(`ret_kind_result_width(TaggedValue) = 2`); every other
supported kind contributes 1.

---

## 7. Open Questions / Known Gaps

Listed in Codex review priority order (post-ADR-0079):

1. **`pairs(t)` hash iteration** (LIC-2.8e-iter-pairs-1).
   Tagged-key layout (ADR 0079) is in place; the remaining
   work is the hash-walk protocol with tombstone awareness.
2. **Future indirect-call re-enablement** (signature side
   table, ADR 0075 superseder candidate). Prerequisite for
   generic-for protocol (LIC-2.8e-iter-generic-1).
3. **Full closures** (`2.5c-full`). Heap-allocated environments.
   The general problem of which closure-in-tables (LIC-2.6c-
   tag-hetero-closure-escape-1) is a subset.
4. **Closure-with-upvalues in tables**
   (LIC-2.6c-tag-hetero-closure-escape-1). HIR rejects today
   via the existing escape analysis (ADR 0044 + ADR 0071).
   Best tackled after #3 because it's a special case of the
   same underlying GC/escape design.
5. **Hash key runtime diagnostics** (LIC-2.6b-hash-key-nil-
   runtime-1 / -nan-runtime-1). Dynamic `nil` and `NaN` keys
   via TaggedValue locals: improve the diagnostic surface
   (specific "table index is nil/NaN" error) without changing
   the static reject behaviour.

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
| 0070 | 2.6c-tag-consumers-inline    | Inline `type(t[k])` / `tostring(t[k])` runtime tag dispatch        |
| 0071 | 2.6c-tag-fn-tbl              | Closure-less Function and Table values in tables (TAG_FUNCTION/TABLE) + 4 consumer dispatch chains extended; `emit_inline_index_into_tagged_tmp` Tidy First; closure-with-upvalues HIR-rejected |
| 0072 | 2.6c-tag-fn-tbl-call         | Call a Function value retrieved through a tagged slot (`local g = t[k]; g()`) — TaggedValue arm in `Callee::Indirect` + `emit_value_slot_check_function` trap helper |
| 0073 | 2.6c-tag-rs-split            | 2-layer codegen module split — `primitive.rs` (pure MLIR helpers + `Types`) + `tagged.rs` (tag constants, store/check helpers, pure-tag consumer dispatchers); `emit.rs` 8464 → 6856 LOC |
| 0074 | 2.6c-tag-locals-fn           | Function-return TaggedValue widening — heterogeneous return paths widen `_ret_value_N` slot to TaggedValue; `ret_mlir_types` maps TaggedValue → `(i64 tag, i64 payload_raw)`; new helpers `emit_call_user_into_tagged_slot` / `_tmp` for caller-side result packing; HIR rejects storing tagged-return functions in tables |
| 0075 | 2.6c-tag-callee-arity        | TaggedValue indirect call HIR-rejected (Strict Plan C, supersedes ADR 0072 in part) — `args.len()` arity reconstruction was unsound; LIC-callee-arity-1 + locals-fn-indirect-1 resolved by removal; `emit_value_slot_check_function` deleted |
| 0076 | 2.6c-tag-locals-fn-multi     | Multi-position TaggedValue caller-side walker — new `ret_kind_result_width` / `flat_result_index` / `emit_pack_tagged_result_at_pos` helpers generalise `emit_multi_assign_from_call` to handle multi-position TaggedValue ABI (`(i64, i64, i64, i64)` for two TaggedValue positions); LIC-locals-fn-multi-1 resolved |
| 0077 | 2.7p-arith-string-coerce     | String → Number arith coercion — HIR `ArithStringCoerce` wraps String operands of arith / bitwise BinOps; codegen `emit_tonumber_for_arith` reuses `emit_tonumber`'s sscanf path then promotes NaN sentinel to runtime trap (`s_arith_coerce_failed`); 12 arith / bitwise ops accept String operands; hex floats work via glibc's sscanf%lf; LIC-arith-coerce-1 resolved |
| 0078 | 2.8e-iter-ipairs             | `for k, v in ipairs(t) do … end` parser sugar (Plan C) — new `Keyword::In`, `StmtKind::ForIpairs`, parser branch + `unwrap_ipairs_call` restrict iter form to `ipairs(table)`; HIR desugars to `Block { LocalInit; While { LocalInit IndexTagged; If IsNil → break; BODY; idx += 1 } }` using existing primitives; codegen unchanged; `pairs` and generic-for protocol remain LIC-tracked pending the ADR 0075 indirect-call reopening |
| 0079 | 2.6b-hash-keys               | Hash key kinds expansion (Plan E tagged-key) — hash entry widens 24→32 bytes with `{16-byte tagged key, 16-byte tagged value}`; new `TAG_DELETED=6` retires the `HASH_DELETED_KEY=1` ptr sentinel; new helpers `emit_build_search_key_slot`, `emit_hash_key_hash_dispatched`, `emit_hash_key_eq_dispatched` route 5-kind keys (Number / String / Bool / Function / Table) through the same probe; LIC-2.6a-arr-3 resolved (was partial) |
