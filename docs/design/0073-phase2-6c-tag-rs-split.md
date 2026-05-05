# 0073. Phase 2.6c-tag-rs-split: 2-Layer Codegen Module Split

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

After ADR 0072, `src/codegen/emit.rs` was 8464 LOC. Tagged-value
helpers (~2000 LOC) and pure MLIR plumbing (~600 LOC) sat next to
HIR lowering, table/hash logic, builtin dispatch, and string
materialisation. Codex post-ADR-0072 review put a module split at
the top of the queue:

> 次の function-return widening は HIR/ABI/codegen をまたぐ
> 大きい変更で、今の emit.rs 密度のまま入れるとレビューも切り
> 戻しも重くなります。4 回 defer 済みなら、ここで Tidy First
> として切る価値が高いです。

ADR 0067 / 0069 / 0070 / 0071 / 0072 each weighed and deferred a
`tagged.rs` split. The recurring objection was CA-shaped: a naive
split would force a wholesale `pub(crate)` of unrelated `emit.rs`
internals (the helpers happened to be co-located but were not part
of the same layer).

## Decision

A **2-layer split** with one-way dependencies:

```
emit.rs ──┬─→ tagged.rs ──┐
          │               ├─→ primitive.rs ──→ melior
          └───────────────┘
```

- **`primitive.rs`** holds pure MLIR / LLVM-dialect wrappers —
  thin shells over a single op (load / store / GEP / cast /
  addressof / printf / exit / libc call). No Lua semantics.
  Both `emit.rs` and `tagged.rs` depend on it.
- **`tagged.rs`** holds the TaggedValue runtime model: tag-space
  constants, slot layout constants (`ARRAY_ELEM_OFF_VALUE`,
  `ARRAY_ELEM_SIZE`), the per-tag store / check helpers, the
  defensive trap, and the dispatcher consumers that work purely
  off a tagged-slot pointer (Print / Type / Eq Local-Local).
  Depends only on `primitive.rs` and `crate::hir::ValueKind`.
- **`emit.rs`** keeps everything else: HIR lowering driver,
  table / hash / array / string codegen, builtin entry dispatch,
  control-flow lowering, function emission, statement-context
  tagged materializers (`emit_local_init_tagged`,
  `emit_inline_index_into_tagged_tmp`, `emit_isnil_index`) that
  recurse through `emit_expr`.

`primitive.rs` and `tagged.rs` are declared as `pub(crate) mod`
in `src/codegen/mod.rs`. The crate-public API
(`compile`, `emit_module`, `new_context`) is unchanged.

### What moved to `primitive.rs` (~344 LOC, 13 items)

| Item | Origin | Call sites | Notes |
|------|--------|------------|-------|
| `Types<'c>` struct | emit.rs:135 | every codegen path | Fields `pub(crate)` so `tagged.rs` can read `types.f64` etc. |
| `emit_load` | emit.rs:1622 | 86 | `llvm.load` |
| `emit_store` | emit.rs:1609 | 49 | `llvm.store` |
| `emit_byte_offset_ptr` | emit.rs:3603 | 59 | `llvm.getelementptr i8` const offset |
| `emit_byte_offset_ptr_dynamic` | emit.rs:3627 | 14 | runtime-offset variant |
| `emit_unrealized_cast` | emit.rs:724 | 8 | `builtin.unrealized_conversion_cast` (ADR 0019) |
| `emit_addressof` | emit.rs:7091 | 52 | `llvm.mlir.addressof @global` |
| `emit_printf` | emit.rs:7109 | 14 | `printf(fmt, value)` libc call |
| `emit_exit_with_message` | emit.rs:6999 | 11 | `printf + exit(1)` (ADR 0033) |
| `emit_libc_call_with_result` | emit.rs:4624 | base | parameterised libc call |
| `emit_libc_call_i64` / `_i32` / `_ptr` / `_void` | emit.rs:4557+ | ~20 combined | result-type variants |

### What moved to `tagged.rs` (~1337 LOC, 17 items)

**Constants (8):** `TAG_NIL`, `TAG_NUMBER`, `TAG_BOOL`,
`TAG_STRING`, `TAG_FUNCTION`, `TAG_TABLE`, `ARRAY_ELEM_SIZE`,
`ARRAY_ELEM_OFF_VALUE`.

**Slot allocator (1):** `emit_alloca_slot_for_kind` — only
`TaggedValue` produces a 16-byte slot; the per-kind branching
belongs to the tagged layer.

**Store helpers (7):** `emit_value_slot_store_number` / `_nil` /
`_bool` / `_string` / `_function` / `_table` /
`emit_value_slot_store_dispatched`.

**Tag check / trap (3):** `emit_value_slot_check_number` (ADR
0063), `emit_value_slot_check_function` (ADR 0072),
`emit_tagged_unknown_tag_trap` (ADR 0069).

**Pure-tag consumer dispatchers (3):** `emit_print_tagged_local`
(ADR 0064/0065), `emit_tagged_eq_local_local` (ADR 0066),
`emit_type_tagged_local` (ADR 0067). Each takes a tagged-slot
pointer and dispatches purely off the runtime tag, calling only
primitives and other tagged helpers.

### What stays in `emit.rs` (intentional)

Five tagged-related helpers stay in `emit.rs` because they
depend on `emit_expr` (the HIR expression dispatcher) or
`emit_tostring` (a static-kind Lua helper) — moving them would
force `pub(crate)` on those higher-level entry points, which is
exactly the CA concern that blocked the previous splits:

- `emit_local_init_tagged` (ADR 0063) — recurses through
  `emit_expr` to lower `target` and `key`; uses table / hash
  codegen helpers.
- `emit_inline_index_into_tagged_tmp` (ADR 0071, Tidy First) —
  thin wrapper over `emit_local_init_tagged`; moves with it.
- `emit_isnil_index` (ADR 0061) — recurses through `emit_expr`,
  uses `emit_table_array_buf` / `emit_array_elem_ptr`.
- `emit_tagged_eq_runtime_dispatch` (ADR 0066) — recurses
  through `emit_expr` for both operands.
- `emit_tostring_tagged_local` (ADR 0067) — calls the
  static-kind `emit_tostring` for the Number arm's `%g`
  formatting.

Promoting these to `tagged.rs` is a **follow-up** that pairs
naturally with the next refactor wave (e.g. lifting the
HIR-recursive helpers out, or splitting the static-kind
`emit_tostring` into a third sibling module).

## Alternatives Considered

- **Single `tagged.rs` module without `primitive.rs`.** The
  previously-defaulted approach. Forces tagged.rs to depend on
  `pub(crate)` shells of `emit_load` / `emit_store` /
  `emit_byte_offset_ptr` exposed back into emit.rs's namespace —
  the visibility leakage the user flagged across 5 prior
  defers. Rejected.
- **Move every tagged-named helper at once.** Would also drag
  `emit_expr` and `emit_tostring` into `pub(crate)` because of
  the recursive calls in `emit_local_init_tagged` and
  `emit_tostring_tagged_local`. Rejected — same CA concern.
- **Common dispatch skeleton extraction (`emit_tag_dispatch`
  callback-based).** Codex review (Explore Agent #2) and the
  plan agent both agreed this is worth doing but **not** as
  part of the split — the 5 dispatchers vary in return type
  (`()` / `Value(i1)` / `Value(ptr)`) and per-tag arm coverage
  enough that the abstraction is non-obvious. Defer to a
  separate Tidy First once the split has settled.

## Consequences

- **emit.rs:** 8464 → 6856 LOC (−19%). Concern density
  dropped: the file now reads as "HIR lowering driver +
  table/hash/string + builtin dispatch + statement-context
  tagged materializers" instead of "everything codegen-related."
- **tagged.rs:** 1337 LOC. Self-contained TaggedValue value
  model — a contributor adding a new tag value (or a new
  consumer that dispatches purely off a tagged-slot pointer)
  edits one file.
- **primitive.rs:** 344 LOC. Pure MLIR plumbing. No Lua
  semantics. A contributor can extend the libc-call surface
  here without touching tagged or HIR-coupled code.
- **858 → 858 tests** — refactor only, no semantics change. No
  LIC entries open or close.
- **Public API surface unchanged:** `lumelir::codegen::compile`
  is still the only public entry. The new modules are
  `pub(crate)` so external crates / tests see no difference.
- **Future splits become cheaper.** Once HIR-recursive helpers
  (`emit_local_init_tagged`, `emit_isnil_index`,
  `emit_tagged_eq_runtime_dispatch`) get a co-located home —
  e.g. when `emit_tostring` itself moves into a third sibling —
  the remaining tagged helpers can follow without further
  visibility churn.

## Documentation updates

- [x] §1 slot layout — n/a (constants migrated to tagged.rs but
      values unchanged).
- [x] §2 producer / source taxonomy — n/a (no producer change).
- [x] §3 consumer coverage matrix — n/a (no behaviour change;
      §6 cross-reference notes the new module home).
- [x] §4 LIC consolidation — n/a (no LIC entry opened or
      closed).
- [x] §5 runtime tag invariants — n/a (invariants unchanged).
- [x] §6 producer-consumer cross-reference — appended a
      "Module layout" subsection describing the
      `emit.rs` / `tagged.rs` / `primitive.rs` split so
      contributors know where to place new helpers.
- [x] §7 open questions — re-prioritised: function-return
      widening promoted to #1 now that the split has unblocked
      it; dispatch-skeleton extraction reframed as #2 (no
      longer "deferred forever").
- [x] §8 ADR index — ADR 0073 row added; "Last updated" bumped
      to "after ADR 0073".

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068). No entries change in this ADR.
