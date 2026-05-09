# Tagged Value Semantics

> **Single Source of Truth** for the TaggedValue runtime
> representation introduced across Phase 2.6c (ADRs 0061–0067).
> Update this page whenever a sub-phase changes producer /
> consumer / tag semantics. ADRs continue to record *decisions*;
> this page records *current state*.

**Last updated:** 2026-05-07 (after closure feasibility spike)

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
| Closure with upvalues                       | Stored as cell ptr in tagged slot (`TAG_FUNCTION` payload). Heap-allocated cell + heap-allocated upvalue boxes survive any escape. Dispatch chain compares `cell.fn_ptr == @user_fn_X` and threads the cell ptr into the call's first arg | ADR 0083 Commit 3c |

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
| LIC-2.8e-iter-pairs-1             | `for k, v in pairs(t) do … end` HIR-desugar via `Builtin::Next` + `@__lumelir_next` (refactored from ADR 0080's opaque codegen walker) | 0080 / 0081 |
| LIC-2.8e-builtin-multi-return-1   | Builtin callees with multi-position return signatures; `MultiAssignFromCall` extended through `Callee::Builtin(b)` + `Builtin::ret_kinds()` | 0081 |
| LIC-2.5x-callee-dispatch-1        | TaggedValue local indirect call via per-call-site static dispatch chain (tag-check + ptr-match + direct `func.call @user_fn_X`); reopens `LIC-2.6c-tag-hetero-fn-tbl-call-1` ("resolved by removal" → "resolved by safe static dispatch") | 0082 |
| LIC-2.8e-pairs-tagged-key-write-1 | `t[k] = …` inside a `pairs` body where `k` is the iterator-bound TaggedValue local — codegen runtime tag dispatch (`TAG_NIL` trap, hash probe via the existing tag-aware helpers), Index read on the same shape | 0084 |
| LIC-2.8e-iter-generic-1           | `for k, v in iter, state, ctl do … end` — Phase 1 scope: non-capturing user fn, builtin `next`, function alias. Closure-as-iter rejected via the existing `f.upvalues.is_empty()` filter; lifts automatically when ADR 0083 ships | 0085 |
| LIC-2.6b-hash-key-nan-runtime-1   | NaN cannot be used as a table index (Lua spec §3.4.5). Static Number-key array path (`t[0/0]`) and TaggedValue-key hash probe entry both gated on `cmpf Une` self-self preflight; trap surface is the dedicated `s_table_index_nan` global | 0086 |

### Partial

(none)

### Pending

| ID                                          | Behaviour                                                             | Notes                          |
|---------------------------------------------|-----------------------------------------------------------------------|--------------------------------|
| LIC-2.7p-arith-coerce-tagged-1              | TaggedValue operand arith coerce (`local x = t[1]; print(x + 1)` when x is runtime String) | HIR can't statically resolve the kind; current TaggedValue-arith path traps on non-Number tag (ADR 0063). Unlocking needs runtime tag dispatch in arith codegen |
| LIC-2.6b-hash-key-nil-runtime-1             | Dynamic `nil` hash key via TaggedValue local — runtime probe currently fires the generic missing-key trap; Lua spec wants a specific "table index is nil" diagnostic. ADR 0084 surfaces this for IndexAssign / Index via `s_table_index_nil`; remaining gap is the trap surface for hash-keyed reads through other producers | 0079 / 0084 (partial) |

**Total:** 27 LIC entries — 27 resolved, 0 partial, 1 pending core + 1 pending runtime-diag (LIC-2.6c-tag-hetero-closure-escape-1 resolved by ADR 0083 Commit 3c, 2026-05-10).

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

### Iteration: `pairs` via `next` builtin (ADR 0081)

`for k, v in pairs(t) do BODY end` HIR-desugars to a
`MultiAssignFromCall(Builtin::Next)` inside a `while true` loop:

```text
do
  local __t = TABLE
  local __ctl = nil   -- TaggedValue
  local _broken_N = false
  while true do
    local k, v = next(__t, __ctl)   -- Builtin::Next
    if IsNil(k) then _broken_N = true
    else BODY ; __ctl = k end
  end
end
```

`Builtin::Next` is the first builtin to declare a multi-position
return signature: `ret_kinds() = [TaggedValue, TaggedValue]`. The
codegen materialises this into a `func.call @__lumelir_next` whose
4-i64 result follows ADR 0076's flattened TaggedValue ABI: `(k_tag,
k_payload, v_tag, v_payload)`.

`@__lumelir_next` is a module-level helper emitted unconditionally
per module. Body: a stateless linear scan with a `found` flag.
Phase 1 walks `i = 1..=len` and Phase 2 walks `bi = 0..hash_cap`,
both gated on a `done` alloca that flips once a result is recorded.
The flag starts true when `prev_k == nil` (first call); subsequent
calls flip it from false to true the moment the walker visits the
slot whose tag/payload match `prev_k`. The next live slot after
that is the answer.

**Body-driven mutation safety**: because `__lumelir_next` runs as
a single function call per iteration, each call freshly reads
`header.hash_buf` and `header.array_buf`. The ADR 0080 ptr-
equality guards are no longer needed — a rehash that frees the old
buffer between calls is handled implicitly by the next call's
header reload. Iteration order is unspecified after such mutation,
matching Lua spec.

**Cost**: `next(t, k)` is O(N) per call (linear scan of the entire
table to find the resume point and the next live slot), so a full
`pairs` loop is O(N²). For typical small Lua tables this is fine;
a future Tidy First can swap the linear scan for a bucket-resume
via the existing dispatched probe (ADR 0079) if benchmarks demand.

### Indirect dispatch via static candidate chain (ADR 0082)

`local g = t[1]; g(args)` where `g` is a TaggedValue local
re-enables after ADR 0075's blanket reject via per-call-site
**static dispatch**. HIR `lower_call` enumerates user functions
whose `(param_kinds, ret_kinds)` match the call site, stores the
set in `Callee::IndirectDispatch { local_id, sig, candidates }`,
and codegen emits:

1. **Tag check**: load tag at slot+0; trap with
   `s_call_non_function` if `≠ TAG_FUNCTION`.
2. **Payload load**: `!llvm.ptr` at slot+8.
3. **Dispatch chain**: nested `scf.if` over candidates. Each
   level compares the loaded ptr to `func.constant @user_fn_X`;
   on match emits a *direct* `func.call @user_fn_X(args)` —
   never `func.call_indirect` with a reconstructed cast (Codex
   forward-edge integrity).
4. **Unmatched fall-through**: `s_call_unknown_fn_ptr` trap.
   Defensive backstop unreachable from Lua source today.

Multi-value position (`local k, v = g(...)`) flows through the
same `Callee::IndirectDispatch` shape; `lower_local_multi`
re-runs candidate filtering with `names.len()`-aware ret_kinds
before reaching codegen, and `emit_multi_assign_from_indirect_
dispatch` packs the dispatch chain's flat result vector via
the existing `flat_result_index` walker (ADR 0076).

Function parameters (Phase 2.5b.2) keep the old `Callee::
Indirect(LocalId)` path — their static `Function(arity)` kind
gives a safe direct `func.call_indirect` without a candidate
chain.

### TaggedValue-key IndexAssign / Index (ADR 0084)

`t[k] = v` and `local x = t[k]` where `k` is a TaggedValue local
(typically the iterator binding from `for k, v in pairs(t) do … end`)
route through the runtime-tag-dispatched hash path:

1. The local's existing slot at `slots[idx]` is already a 16-byte
   tagged search-key slot — we hand it directly to the probe, no
   fresh `emit_build_search_key_slot` tmp.
2. Tag check first: `slot+0 == TAG_NIL` ⇒ exit with
   `s_table_index_nil` (Lua spec §3.4.5). Forward-edge integrity
   discipline carried over from ADR 0082.
3. Hash probe via the existing tag-dispatched helpers
   (`emit_hash_key_hash_dispatched` / `emit_hash_key_eq_dispatched`,
   ADR 0079). No per-tag specialisation at the call site.
4. Write-side new-key commit: raw 16-byte copy of the search slot
   (tag + payload) into `entry+0`. The slot's words are already in
   `{i64 tag, i64 payload}` shape, so no kind-aware store is
   required.

The array path is bypassed entirely — TaggedValue Number-tagged
keys are written to / read from the hash mirror, never the array.
This is a documented trade-off: a future ADR may unify the
read-side dispatch so static-Number reads check the hash mirror
after the array slot.

### Generic-for protocol (ADR 0085)

`for k, v in ITER, STATE, CTL do BODY end` HIR-desugars to a
synthetic block that pins state / ctl / iter to fresh locals and
calls iter each iteration through whichever `Callee` shape the iter
resolves to:

| Source iter shape                                 | Callee                              |
|---------------------------------------------------|-------------------------------------|
| `Ident("next")`                                   | `Builtin::Next`                     |
| `FunctionRef(fid)`                                | `User(fid)`                         |
| `Local(idx)` / `Function(arity=2)` with FuncId    | `User(fid)`                         |
| `Local(idx)` / `Function(arity=2)` parameter      | rejected (single-Number ret ABI)    |
| `Local(idx)` / `TaggedValue`                      | `IndirectDispatch { sig, candidates }` (post-ADR 0083 Commit 3c: candidate set includes capturing closures because the dispatch chain threads each candidate's cell ptr through the cell-ptr-first ABI) |

ADR 0083 Commit 3c (2026-05-10) removed the `f.upvalues.is_empty()`
filter. The iter must return `(TaggedValue|Nil, _)` so the loop
can receive `nil` as the termination sentinel — Number-only or
Bool-only first ret_kind is rejected at HIR (would loop forever).

### Hash key NaN trap (ADR 0086)

NaN cannot be a table index (Lua spec §3.4.5). NaN preflight is
inserted at four sites; each runs `cmpf Une key key` (true iff
NaN, agnostic to qNaN / sNaN / ±NaN) and exits with the dedicated
`s_table_index_nan` global on the then branch:

| Site                                     | Condition                                      |
|------------------------------------------|------------------------------------------------|
| `IndexAssign` Number-key arm             | static Number key, before `f2i` / bounds-check |
| `Index` Number-key arm                   | static Number key, before `f2i` / bounds-check |
| `emit_local_init_tagged` Number-key arm  | inline `print(t[expr])` / `tostring(t[expr])`  |
| `emit_hash_probe_loop` entry             | TaggedValue keys, only when tag == TAG_NUMBER  |

The fourth site (probe loop entry) is the single chokepoint for
both `emit_hash_probe_for_insert` and `emit_hash_probe_lookup`;
one preflight here covers every TaggedValue-key call site
(IndexAssign / Index / iterator-internal probes) without
duplicating the check. `cmpf Une self-self` was reused from
`emit_tonumber_for_arith` (ADR 0077). Diagnostic stays distinct
from `s_table_index_nil` (ADR 0084) and `s_table_missing_key`
(ADR 0079) — three layered traps for three layered failure modes.

---

## 7. Open Questions / Known Gaps

Listed in Codex review priority order (post-ADR-0082):

1. **Full closures** (`2.5c-full`). Heap-allocated environments.
   The general problem of which closure-in-tables (LIC-2.6c-
   tag-hetero-closure-escape-1) is a subset. Commit 2a / 2a-fix /
   2b have landed (2026-05-07/08); only Commit 3 (captured-local
   boxes) remains:
   - **Commit 2a** (`551d51c`): `emit_function` / `emit_main` /
     `emit_lumelir_next_function` migrated to
     `LLVMFuncOperationBuilder`, multi-return wrapped in
     `!llvm.struct<(...)>` per the B5b spike. Function-kind
     param/ret types went from `!func.func<...>` to
     `!llvm.ptr`. 965/0 unchanged.
   - **Commit 2a-fix** (`c81f16b`): HIR-level reject of
     non-Number ret_kinds on Function-kind parameter routes,
     restoring the verifier safety net that the `!llvm.ptr`
     erasure removed (ADR 0075 amend). 5 reject tests added,
     970/0.
   - **Commit 2b**: per-user-fn static `@user_fn_NN_closure`
     globals (16-byte `!llvm.struct<(ptr, i64)>` initialised to
     `{addressof @user_fn_NN, 0}`). TAG_FUNCTION payload is
     now a closure cell ptr; producers materialise the cell
     via `addressof @<fn_sym>_closure`, consumers normalise
     back to fn ptr via `closure::emit_load_closure_fn_ptr`
     before the actual `llvm.call`. The dispatch-chain
     candidate side stays at raw fn ptr (no double indirection).
     Singleton property (1 fn = 1 global) preserves Lua spec
     §3.4.4 closure equality without extra work. 3 new
     IR-shape tests, 973/0.
   - **Commit 3a** (`20e563e`): `closure.rs` 6 capturing
     helpers (`emit_allocate_closure_cell`,
     `emit_allocate_upvalue_box`, `emit_load/store_closure_upvalue_box`,
     `emit_load/store_upvalue_box_value`); `LocalInfo::is_captured`
     + post-pass; `HirFunction::parent_scope`. 978/0.
   - **Commit 3b prep / prep fix** (`e8db350` / `f2ffcb9`):
     `Callee::User` struct variant `{ fid, holding_local }`,
     `emit_call_user_with_cell` helper, synthetic
     FunctionDef-locals, post-pass `MutualCapturingRecursion`
     reject. 980/0.
   - **Commit 3b body atomic** (`18bee17`): cell-ptr-first
     ABI on every user `llvm.func`; 4 direct-call sites
     unified through `emit_call_user_with_cell`; FunctionRef
     allocates capturing cells; LocalInit storage rule for
     capturing targets; outer-scope `is_captured` heap boxes
     pre-allocated at function entry. 984/0.
   - **Commit 3c**: removed all 5 `HirError::ClosureEscapes`
     reject sites + `closure_with_upvalues` helper +
     `f.upvalues.is_empty()` generic-for filter; threaded
     loaded cell ptr (not `cell.fn_ptr`) as
     `in_function_cell_ptr` through `Callee::Indirect` and
     dispatch chain so capturing closures reach their boxes
     when reached via tagged-slot escape paths; 7 e2e tests
     pin box_sharing / make_adder / closure_return /
     table_capture / closure_identity / generic_for_capturing /
     IR-shape; 7 negative escape tests across 6 files
     inverted to positive lowering pins. 990/0. Resolves
     LIC-2.6c-tag-hetero-closure-escape-1; ADR 0044 fully
     superseded.
3. **TaggedValue arith coerce** (LIC-2.7p-arith-coerce-tagged-1).
   ADR 0077's String → Number arith coerce only fires when the
   String operand is statically typed; a TaggedValue local that
   holds a String at runtime (`local x = t[1]; print(x + 1)`)
   still traps on the non-Number tag check. Resolution requires
   runtime tag dispatch in arith codegen, parallel to the
   consumer dispatchers added in ADRs 0067 / 0070.
4. **Hash key nil runtime diagnostic completion**
   (LIC-2.6b-hash-key-nil-runtime-1). ADR 0084's
   `s_table_index_nil` covers IndexAssign / Index for TaggedValue
   locals; the gap is the trap surface for hash-keyed reads
   produced by other shapes (closure-return locals, table-value
   locals once they're lifted out of the current HIR rejection).
   ADR 0086's NaN preflight is the structural template for the
   nil completion when those producer surfaces open.

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
| 0080 | 2.8e-iter-pairs              | `for k, v in pairs(t) do … end` dual-phase codegen walker — parser + HIR sibling of ForIpairs; codegen `emit_for_pairs` walks array part 1..=len then hash part 0..cap with tombstone (`TAG_DELETED`) skip; per-iteration `header.hash_buf` / `header.array_buf` reload + ptr-equality detect aborts on body-driven rehash (Codex pre-review P1); new helper `emit_copy_value_slot_16b` consolidates the rehash-migration copy pattern; LIC-2.8e-iter-pairs-1 resolved; new pending LIC-2.8e-pairs-tagged-key-write-1 (TaggedValue key IndexAssign HIR-rejected) |
| 0081 | 2.8e-iter-next               | `next(t, k)` builtin + ForPairs HIR-desugar (Plan Alpha, Codex post-ADR-0080) — `Builtin::Next` is the first multi-return builtin; `Builtin::ret_kinds()` + `MultiAssignFromCall(Callee::Builtin)` open the path. Module-level `@__lumelir_next` (stateless `(t, prev_k) → (k, v)` scan with linear find/resume) replaces ADR 0080's `emit_for_pairs` walker; ForPairs lowers to `Block + LocalInit + While + MultiAssignFromCall + If + Assign`. ~707 LOC of codegen deleted (`emit_for_pairs` and 4 helpers); ~750 LOC added (`__lumelir_next` body + multi-assign-from-builtin + extract-prev-k). 5 new e2e in `tests/phase2_8e_next.rs`, 16 ADR 0080 e2e regress green. LIC-2.8e-iter-pairs-1 resolution mechanism updated; new resolved LIC-2.8e-builtin-multi-return-1. 22/0/4 |
| 0082 | 2.5x-callee-dispatch         | General indirect-call re-enablement (Plan B3, Codex post-ADR-0081, supersedes ADR 0075 in part) — `Callee::IndirectDispatch { local_id, sig: IndirectSig, candidates: Vec<FuncId> }` extends `Callee` (kept `Indirect` for parameter calls). HIR `lower_call` filters user fns by `param_kinds`, picks the first match's `ret_kinds` as canonical, and re-runs `compatible_user_functions` for full-sig candidates; `lower_local_multi` / `lower_assign_multi` re-search for multi-value position. Codegen `emit_indirect_dispatch_call` does (1) tag-check vs `TAG_FUNCTION` with `s_call_non_function` trap, (2) ptr load at slot+8, (3) nested `scf.if` chain comparing `loaded_ptr` to each candidate's `func.constant @user_fn_X` and emitting **direct** `func.call @user_fn_X(args)` (no `func.call_indirect` cast — Codex forward-edge integrity). New `src/codegen/callabi.rs` extracts `ret_mlir_types` / `ret_kind_result_width` / `flat_result_index` (Tidy First). 11 reframed tests (ADR 0072/0075 reject → positive) + 4 new e2e (multi-return indirect, closure-escape regression, no-candidates compile error, same-sig dispatch). 940 → 944 green. LIC-2.6c-tag-hetero-fn-tbl-call-1 reframed "resolved by safe static dispatch"; new resolved LIC-2.5x-callee-dispatch-1. 23/0/4 |
| 0084 | 2.8e-iter-tk                 | TaggedValue-key IndexAssign + Index read (Codex pivot to (C), ADR 0083 deferred). HIR `is_hash_key_eligible` accepts `ValueKind::TaggedValue`; codegen runtime tag dispatch in IndexAssign / Index passes the local's slot directly to the ADR 0079 hash probe with a `TAG_NIL` trap (`s_table_index_nil`, Lua spec §3.4.5). New-key commit copies the 16-byte search slot into `entry+0` raw. Resolves the natural `for k, v in pairs(t) do t[k] = v + 100 end` idiom; ADR 0080's `pairs_body_writes_separate_table_safely` workaround reframed to `pairs_body_mutates_existing_value_safely`. 7 new e2e + 1 reframe, 944 → 951 green. LIC-2.8e-pairs-tagged-key-write-1 resolved; LIC-2.6b-hash-key-nil-runtime-1 noted as partial via the new trap surface. 24/0/3 |
| 0085 | 2.8e-iter-generic            | Full Lua 5.4 §3.3.5 generic-for parser sugar — `for k, v in ITER, STATE, CTL do BODY end`. New `StmtKind::ForGeneric { names, iter, state, ctl, body }` parser variant + `IterMatch::Generic` discriminator; HIR synthetic-block desugar pins state / ctl / iter to fresh locals and dispatches the per-iteration call through `Callee::Builtin(Next)` / `User(fid)` / `IndirectDispatch` based on iter's resolved shape. Phase 1 scope filters closure-as-iter via `f.upvalues.is_empty()` (carries over to ADR 0083 follow-up). Iter must return `(TaggedValue\|Nil, _)` so a `nil` first result can terminate. 8 new e2e in `tests/phase2_8e_generic_for.rs`, 951 → 959 green. LIC-2.8e-iter-generic-1 resolved (Phase 1). 25/0/3 |
| 0086 | 2.6b-hash-key-nan            | Hash key NaN runtime diagnostic (Codex pivot from ADR 0083 deferral) — Lua spec §3.4.5 forbids NaN as a table index. New `s_table_index_nan` global + `emit_table_index_nan_trap_if` / `emit_hash_key_nan_preflight` helpers. NaN preflight inserted at 4 sites: static Number-key IndexAssign / Index arms (before `f2i`), inline `emit_local_init_tagged` Number-key arm (covers `print(t[0/0])`), and `emit_hash_probe_loop` entry (single chokepoint for every TaggedValue-key call). `cmpf Une self-self` reused from ADR 0077's `emit_tonumber_for_arith` — qNaN/sNaN/±NaN agnostic. 6 new e2e in `tests/phase2_6b_hash_key_nan.rs`, 959 → 965 green. LIC-2.6b-hash-key-nan-runtime-1 resolved. 26/0/2 |
