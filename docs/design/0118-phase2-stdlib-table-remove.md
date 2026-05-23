# 0118. Phase 2.7r-stdlib-table: `table.remove(t [, pos])` mutation primitive

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-23 (commit `4248891`)
- **Deciders:** ShortArrow

## Replan provenance

ADR 0117 (`emit_print_string_obj` fwrite swap) repaired the ADR
0112 8-bit-clean stdout carry-over. Codex post-0117 6-視点
verdict for the next step was **B (`table.remove`) = Best Go**
on Phase 2 completion grounds (over A `io.read` / G math
constants / K OOM consolidation / bundles), with the rationale
that `table.remove` is the direct symmetric extension of ADR
0111 `table.insert` and reuses ADR 0114's integer gate
unchanged.

`table.remove` is the **first `table.*` builtin with a non-void
return** — `ret_kinds = [TaggedValue]` because the removed
element can be any Lua kind. ADR 0118 also adds the
`Callee::Builtin` arm to `emit_local_init_tagged` so `local v =
table.remove(t)` flows through the standard local-init path.

## Codex critical fixes baked in

1. **Mirror of `table.insert`** — `emit_table_remove_runtime`
   is the direct counterpart of `emit_table_insert_runtime`:
   memmove shift-LEFT instead of shift-right, no grow path
   (shrink only).
2. **First table.* with non-void return** — `ret_kinds =
   [TaggedValue]` because removed elements are heterogeneous.
3. **`emit_local_init_tagged` extension** for `Callee::Builtin`
   returning TaggedValue (mirror of the existing `Callee::User`
   arm from ADR 0074).
4. **Lua spec edge case** — `pos == #t + 1` is a no-op that
   returns nil; empty table + default `pos = #t = 0` traps
   (matches reference impl, not the more permissive Lua manual
   wording).
5. **No bundle** — codex critical: B+H (table.remove + io.flush)
   splits lanes; A+H mixes io effect with feature lane.

## Non-goals (top-of-ADR)

- **`print(table.remove(t))` direct** — Print/io.write
  Builtin-TaggedValue source dispatch is a separate ADR.
  Workaround: `local v = table.remove(t); print(v)`.
- **`table.unpack` / `pack` / `sort` / `move`** — table.*
  mutation suite continuation; separate ADRs each.
- **`io.read` / `io.flush`** — feature lane mismatch.
- **OOM consolidation 全方位** — devinfra Tidy First lane,
  later ADR.
- **`Builtin::ret_kinds` arity-dependent framework** — still no
  trigger; `next` is the only multi-return builtin.
- **`pcall` / `error` value propagation** — Phase 2 完成条件外.

## Goals

1. `Builtin::TableRemove` variant + `table_from_method("remove")`
   arm + `arity (1, 2)` + name "table.remove" + `ret_kinds
   [TaggedValue]`.
2. `param_kinds_for_arity`: arity 1 → `[Table]`, arity 2 →
   `[Table, Number]` (mirror of TableInsert arity-sensitive
   shape).
3. 2 new diagnostic globals: `s_table_remove_pos_out_of_range`
   + `s_table_remove_non_integer`.
4. `emit_table_remove_runtime` helper — bounds check + value
   read + shift-left + length decrement.
5. `Callee::Builtin(TableRemove)` emit arm — allocates tmp
   tagged slot, calls runtime, returns slot ptr.
6. `emit_local_init_tagged` Builtin-TaggedValue arm.
7. Test corpus: 1235 → 1248 (+13).

## Lua 5.4 §6.6 compliance

```lua
table.remove(t)         -- pos default = #t (tail pop)
table.remove(t, 1)      -- head pop with shift-left
table.remove(t, 2)      -- middle pop with shift-left
local v = table.remove(t)  -- v = removed value (TaggedValue)
```

Validity:
- `1 ≤ pos ≤ #t`: normal path, shift-left + return value
- `pos == #t + 1`: no-op, returns nil (Lua manual)
- `#t == 0 AND pos == 0`: traps (reference impl behavior;
  Lua manual is permissive, we follow impl)
- otherwise: trap "position out of bounds"

## 設計

### HIR (`src/hir/ir.rs`)

```rust
enum Builtin {
    TableRemove,
}

impl Builtin {
    pub fn table_from_method(method: &str) -> Option<Builtin> {
        match method {
            "concat" => Some(Builtin::TableConcat),
            "insert" => Some(Builtin::TableInsert),
            "remove" => Some(Builtin::TableRemove),
            _ => None,
        }
    }

    pub fn arity(self) -> (usize, usize) {
        match self {
            Builtin::TableRemove => (1, 2),  // tail-pop or explicit pos
            // ...
        }
    }

    pub fn ret_kinds(self) -> &'static [ValueKind] {
        match self {
            Builtin::TableRemove => &[ValueKind::TaggedValue],
            // ...
        }
    }

    pub fn param_kinds_for_arity(self, argc: usize) -> &'static [ValueKind] {
        match self {
            Builtin::TableRemove => match argc {
                1 => &[ValueKind::Table],
                2 => &[ValueKind::Table, ValueKind::Number],
                _ => &[],
            },
            // ...
        }
    }
}
```

`infer_kind` for TableRemove returns `ValueKind::TaggedValue`
in single-value position.

### `emit_table_remove_runtime` (`src/codegen/emit.rs`)

```rust
fn emit_table_remove_runtime(
    context, block, t_ptr, len_pre, pos_i64, out_slot, types, loc,
) {
    // Step 1: bounds classify
    //   in_range  = (1 <= pos <= len_pre)
    //   no_op_top = (pos == len_pre + 1)
    //   trap on otherwise
    //
    // Step 2: scf::r#if (oob) trap with s_table_remove_pos_out_of_range
    //
    // Step 3: scf::r#if (is_real = pos <= len_pre) {
    //     - load 16-byte slot from array_buf[pos-1] → out_slot
    //     - if pos < len_pre: memmove (len_pre - pos) slots left
    //     - length -= 1
    //   } else {
    //     - out_slot.tag = TAG_NIL
    //     - (length unchanged)
    //   }
}
```

Helper extras:
- `emit_trap_if` (ADR 0114) for the OOB branch
- 16-byte raw load (tag + payload as separate i64) per ADR 0064
- `memmove` extern (already declared by ADR 0111)
- No `emit_table_grow_if_needed` (shrink only)

### `Callee::Builtin(TableRemove)` emit arm

```rust
Callee::Builtin(Builtin::TableRemove) => {
    let t_ptr = emit_expr(... &args[0] ...)?;
    let len_slot = emit_byte_offset_ptr(... TABLE_OFF_LEN ...);
    let len_pre = emit_load(block, len_slot, types.i64, loc);
    let pos_i64 = if args.len() == 1 {
        len_pre   // default pos = #t
    } else {
        let pos_f64 = emit_expr(... &args[1] ...)?;
        // ADR 0114 integer gate (mirror of table.insert)
        emit_check_integer_arg(
            context, block, pos_f64,
            "s_table_remove_non_integer", types, loc,
        )
    };
    let out_slot = emit_alloca_slot_for_kind(
        context, block, ValueKind::TaggedValue, types, loc,
    );
    emit_table_remove_runtime(
        context, block, t_ptr, len_pre, pos_i64, out_slot, types, loc,
    );
    Ok(out_slot)  // tmp tagged slot ptr is the expression result
}
```

### `emit_local_init_tagged` Builtin-TaggedValue arm

Add a new arm alongside the existing `Callee::User` arm
(ADR 0074):

```rust
HirExprKind::Call {
    callee: Callee::Builtin(b),
    ..
} if matches!(b.ret_kinds().first(), Some(ValueKind::TaggedValue)) => {
    let src_slot = emit_expr(...)?;
    let dst_slot = slots[id.0];
    // 16-byte raw-i64 copy (tag + payload) per ADR 0064
    let tag = emit_load(block, src_slot, types.i64, loc);
    emit_store(block, tag, dst_slot, loc);
    let src_pay = emit_byte_offset_ptr(... ARRAY_ELEM_OFF_VALUE ...);
    let dst_pay = emit_byte_offset_ptr(... ARRAY_ELEM_OFF_VALUE ...);
    let payload = emit_load(block, src_pay, types.i64, loc);
    emit_store(block, payload, dst_pay, loc);
}
```

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `Builtin::from_namespace_method` table arm | `src/hir/ir.rs:530` | namespace dispatch |
| `table_from_method` | `src/hir/ir.rs:540` | "remove" → variant |
| `emit_check_integer_arg` (ADR 0114) | `src/codegen/emit.rs:9498` | pos non-integer gate |
| `emit_table_insert_runtime` mirror (ADR 0111) | `src/codegen/emit.rs:10348` | shift-right reference |
| `memmove` extern (ADR 0111) | `src/codegen/emit.rs:1053` | shift-left libc call |
| 16-byte slot copy idiom (ADR 0064) | `src/codegen/emit.rs:4945` | tag + payload as raw i64 |
| `emit_trap_if` (ADR 0114) | `src/codegen/emit.rs:9489` | scf::if + exit_with_message |
| `emit_alloca_slot_for_kind` | `src/codegen/tagged.rs:163` | tmp slot alloc |
| `len_slot_of` (ADR 0111 helper) | `src/codegen/emit.rs:10620-10628` | length field ptr |
| `Callee::User` TaggedValue-return arm (ADR 0074) | `src/codegen/emit.rs` | mirror for Builtin arm |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: direct mirror of
  `table.insert`. No new framework.
- [x] **#2 TDD**: 12 e2e + 1 shadowing Day-0 Green pin.
- [x] **#3 FP**: `ret_kinds` extension stays static slice (no
  arity-dependent multi-result framework). `emit_local_init_
  tagged` Builtin arm mirrors User arm.
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/primitive.rs`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security/integrity**: OOB pos trap via ADR 0114
  integer gate; no new alloc (shrink only); memmove overlap-safe.
- [x] **#6 Documentation**: phase tag `2.7r-stdlib-table` (extends
  ADR 0106 / 0107 / 0108 / 0111 row; no new lane).

## Test count delta

1235 → 1248 (+13 net = 12 Red Day 0 + 1 shadowing Day-0 Green).

`tests/phase2_stdlib_table.rs`:

| Test | Category |
|---|---|
| `table_remove_default_pos_tail_pop` | happy |
| `table_remove_returns_removed_number` | value-return |
| `table_remove_explicit_pos_one_head` | happy |
| `table_remove_explicit_pos_middle` | happy |
| `table_remove_single_element_table` | edge |
| `table_remove_returns_removed_string` | value-return |
| `table_remove_pos_size_plus_one_returns_nil` | spec edge |
| `table_remove_empty_default_pos_traps` | runtime trap |
| `table_remove_pos_zero_nonempty_traps` | runtime trap |
| `table_remove_pos_past_end_plus_one_traps` | runtime trap |
| `table_remove_non_integer_pos_traps` | integer gate |
| `table_remove_non_table_first_arg_hir_rejects` | HIR negative |
| `table_remove_shadowed_respects_user_table` | codex critical (Day-0 Green via index-callee fall-through) |

## Critical files

- `src/hir/ir.rs` (~20 LOC: variant + dispatch + arity + name +
  ret_kinds + param_kinds_for_arity)
- `src/hir/mod.rs` (~1 LOC: infer_kind)
- `src/codegen/emit.rs`:
  - 2 new diagnostic globals (~25 LOC)
  - `emit_table_remove_runtime` helper (~250 LOC)
  - `Callee::Builtin(TableRemove)` emit arm (~50 LOC)
  - `emit_local_init_tagged` Builtin arm (~40 LOC)
- `tests/phase2_stdlib_table.rs` (~170 LOC: 13 e2e)

**Zero-diff (CA invariant)**:
`src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`,
`src/codegen/primitive.rs`, `src/codegen/tagged.rs`.

## Risks

| Risk | Mitigation |
|---|---|
| Builtin-returning-TaggedValue path complex | `Callee::User` precedent (ADR 0074) is direct mirror |
| 16-byte raw copy with wrong tag | ADR 0064 idiom; tag + payload as separate i64 loads |
| `pos == #t + 1` no-op memmove misbehavior | bounds classify branches to no-op TAG_NIL only |
| Empty table + default pos surprises user | Matches Lua reference impl behavior (manual is permissive but impl strict); documented |
| `print(table.remove(t))` direct fails | Non-goal; workaround `local v = ...; print(v)` documented |
| Larger Builtin-TaggedValue source dispatch (Print/io.write/concat) needed | Out of scope; follow-up ADR |

## Future work

- **Print / io.write / concat-arg Builtin-TaggedValue source
  dispatch**: `print(table.remove(t))` direct path.
- **ADR 0119** — `io.read("*l")`.
  **RESOLVED by ADR 0119 (2026-05-23)**.
- **`table.unpack` / `pack` / `sort` / `move`**: table.* mutation
  suite continuation.
- **`string.byte(s, i, j)` multi-byte form**: would trigger
  `Builtin::ret_kinds` arity-dependent framework.
- **OOM consolidation 全方位**: devinfra Tidy First lane.

## Phase tag

`2.7r-stdlib-table` (continues the ADR 0106/0107/0108/0111
lane; no new tag needed).
