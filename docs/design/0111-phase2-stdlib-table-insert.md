# 0111. Phase 2.7r-stdlib-table: table.insert + arity-sensitive param_kinds

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-17
- **Deciders:** ShortArrow

## Replan provenance

ADR 0110 (`4238239`) で namespace stdlib arg-kind validation
policy 完了直後、自然な candidate は Codex prior #2 = `string.char`
だったが、Codex 6-視点 review で **NoGo**: current C-string ABI
(ADR 0024 strlen-based length) が `string.char(0)` / embedded-NUL
を扱えない。

User 合意で 3 ADR sequence:
- **ADR 0111** = ABI-independent feature: **`table.insert`**
- **ADR 0112** = String ABI refactor (ptr+len, + OOM consolidation
  bundle)
- **ADR 0113** = `string.char` proper

本 ADR 0111 は ADR 0112/0113 への bridge — string ABI 独立な
table mutation primitive を投入し、0110 直後の feature delivery
momentum を維持しつつ ABI refactor の準備期間を作る。

Codex post-0110 6-視点 review (table.insert scope) verdict:
**Refactor → Go**、critical 1 件 — `Builtin::param_kinds()` を
arity-sensitive な `param_kinds_for_arity(argc)` に小 refactor
必要 (table.insert は arg 1 が arity 2 では value, arity 3 では
pos で semantics 切り替わる; ADR 0110 の "position-stable" 契約
を満たさない).

```lua
local t = {1, 2, 3}
table.insert(t, 4)              -- append: t = {1,2,3,4}
table.insert(t, 1, 0)            -- head: t = {0,1,2,3,4}
table.insert(t, 3, 99)           -- middle: t = {0,1,99,2,3,4}
table.insert(t, "str")           -- any-kind value: t[7] = "str"
table.insert(t, 100, 1)          -- runtime trap (pos > #t+1)
table.insert(t)                  -- HIR ArityMismatch (arity < 2)
table.insert("str", 1)           -- HIR BuiltinArgKindMismatch
```

## Non-goals (top-of-ADR)

- **`lower_namespace_builtin_call` special-case 復活** — ADR
  0107/0110 で uniform 化したラインを壊す。NoGo。代わりに
  `param_kinds_for_arity(argc)` 小 refactor で arity-sensitive
  対応 (TableInsert のみ argc 分岐; 他全 builtin は argc 無視で
  static slice 返却).
- **arity 2 form の HIR desugar (`t[#t+1] = v`)** — codex critical:
  builtin call は expression / IndexAssign は statement で IR
  ownership 不整合。Builtin::TableInsert を残し codegen で
  arity 2 を `pos = len + 1` に正規化。
- **Shift loop の reverse scf::while 手書き** — codex critical:
  `memmove` libc 直 call が clean (overlap-safe semantics、
  16-byte slot 固定)。memcpy だと overlap UB.
- **Strict Lua spec の pos out-of-range** — Lua spec mandates
  runtime error for pos > #t+1 or pos < 1. 採用 (runtime trap
  with new diagnostic global). pos = #t+1 は valid (append
  equivalent).
- **Value arg の kind 制約** — Lua spec: any value (Nil 含む).
  `param_kinds_for_arity` で `ValueKind::TaggedValue` を "any"
  sentinel として使用; ADR 0110 check は両側の TaggedValue で
  skip するように本 ADR で拡張.
- **Non-integral pos の trap** — MVP では fptosi truncate 許容
  (string.sub / byte と一貫). 将来 arg-validation policy ADR で再考.
- **malloc OOM** — carry-over (ADR 0112 bundle 予定).
- **`s:insert(v)` method syntax** — Phase 3 metatables.

## Lua 5.4 §6.8 spec

```
table.insert(list, [pos,] value)
  arity 2: pos = #list + 1 (append form)
  arity 3: insert at pos (1-based, valid [1, #list + 1])
  pos < 1 OR pos > #list + 1: runtime error
  shift list[pos..#list] right by 1
  list[pos] = value
  #list += 1
returns nothing (void)
```

## 設計

### 1. HIR `Builtin::param_kinds_for_arity(argc)` refactor

**Codex critical**: 旧 `param_kinds()` (position-stable) は
table.insert で破綻。signature を `(self, argc: usize) -> &'static
[ValueKind]` に変更:

```rust
pub fn param_kinds_for_arity(self, argc: usize) -> &'static [ValueKind] {
    match self {
        // table.insert: arity-sensitive
        Builtin::TableInsert => match argc {
            2 => &[ValueKind::Table, ValueKind::TaggedValue],
            3 => &[ValueKind::Table, ValueKind::Number, ValueKind::TaggedValue],
            _ => &[],
        },
        // 他全 builtin は argc 無視 (既存 static slice)
        Builtin::MathSqrt | ... => &[ValueKind::Number],
        Builtin::TableConcat => &[ValueKind::Table, ValueKind::String, ValueKind::Number, ValueKind::Number],
        // (etc.)
    }
}
```

`lower_namespace_builtin_call` caller 更新:
`builtin.param_kinds() → builtin.param_kinds_for_arity(args.len())`

### 2. ADR 0110 check 拡張: `expected == TaggedValue` skip

ValueKind::TaggedValue を param_kinds で "any kind accepted"
sentinel として使うため、check loop 内で expected 側も skip:

```rust
if matches!(actual, ValueKind::TaggedValue)
    || matches!(expected, ValueKind::TaggedValue)
{
    continue;
}
```

これで `table.insert(t, "str")` (value=String) や `table.insert(t,
nil_val)` (value=Nil) が HIR で通過 (Lua spec 通り).

### 3. HIR `Builtin::TableInsert` variant

- arity = `(2, 3)` (6th range-arity builtin)
- name = `"table.insert"`
- ret_kinds = `&[]` (void)
- param_kinds_for_arity: arity-sensitive (上記)
- `table_from_method("insert") → Some(TableInsert)`
- `infer_kind` 拡張: `Callee::Builtin(TableInsert) → Number`
  (Print precedent — void でも expression-position synthesis用)

### 4. Codegen 新 helper `emit_table_insert_runtime`

```rust
fn emit_table_insert_runtime(t_ptr, len_pre, pos_i64, value_expr, ...) {
    // 1. Range check: 1 <= pos <= len_pre + 1
    let max_valid = len_pre + 1
    let oob = (pos < 1) || (pos > max_valid)
    scf::if(oob): trap(s_table_insert_pos_out_of_range)
    // 2. Grow if needed (ADR 0057 reuse)
    emit_table_grow_if_needed(t_ptr, max_valid, len_pre)
    // 3. Reload array_buf
    let arr_buf = load(t_ptr + TABLE_OFF_ARRAY_BUF, ptr)
    // 4. Shift: if pos <= len_pre, memmove
    let pos_zero = pos - 1
    let shift_needed = (pos <= len_pre)
    scf::if(shift_needed):
        memmove(arr_buf + (pos_zero + 1)*16, arr_buf + pos_zero*16, (len_pre - pos + 1)*16)
    // 5. Store value at slot[pos]
    let target_slot = arr_buf + pos_zero * 16
    if value_kind == TaggedValue Local:
        // raw 16-byte copy (preserves Nil tag)
        copy_slot(value_src, target_slot)
    else:
        emit_value_slot_store_dispatched(target_slot, v, value_kind)
    // 6. length += 1
    store(t_ptr + TABLE_OFF_LEN, len_pre + 1)
}
```

### 5. Codegen TableInsert emit arm

```rust
Callee::Builtin(Builtin::TableInsert) => {
    let t_ptr = emit_expr(args[0]);
    let len = load(t_ptr + TABLE_OFF_LEN, i64);
    let (pos_i64, value_expr) = if args.len() == 2 {
        (len + 1, &args[1])  // append
    } else {
        (emit_f2i(emit_expr(args[1])), &args[2])
    };
    emit_table_insert_runtime(t_ptr, len, pos_i64, value_expr);
    // void: placeholder f64 0.0
    Ok(const_f64(0.0))
}
```

### 6. New libc extern `memmove`

`emit.rs` の libc extern 群 (alongside memcpy):

```rust
let memmove_ty = llvm::r#type::function(types.ptr, &[types.ptr, types.ptr, types.i64], false);
LLVMFuncOperationBuilder::new(...).sym_name("memmove").build();
```

### 7. New diagnostic global `s_table_insert_pos_out_of_range`

```rust
emit_string_global(... "s_table_insert_pos_out_of_range",
    "bad argument #2 to 'insert' (position out of bounds)\0", ...);
```

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_table_grow_if_needed` (ADR 0057) | `src/codegen/emit.rs` | grow array_buf to required cap |
| `emit_value_slot_store_dispatched` (ADR 0064) | `src/codegen/tagged.rs` | store any-kind value into 16-byte tagged slot |
| `emit_libc_call_ptr` (memcpy/memmove) | `src/codegen/primitive.rs` | libc shift |
| `emit_byte_offset_ptr` / `_ptr_dynamic` | `src/codegen/primitive.rs` | slot ptr |
| `emit_load` / `emit_store` | `src/codegen/primitive.rs` | header read/write |
| `emit_addressof` | `src/codegen/primitive.rs` | diagnostic ptr |
| `emit_exit_with_message` | `src/codegen/primitive.rs` | trap |
| `emit_string_global` | `src/codegen/emit.rs` | new diagnostic global |
| `emit_f2i` | `src/codegen/emit.rs` | pos f64 → i64 |
| `arith::cmpi(Slt/Sgt/Sle) / addi / subi / muli / ori` | melior 0.27 | int arith + bool combine |
| `scf::r#if` | melior 0.27 | trap branch + shift branch |
| TABLE_OFF_LEN / TABLE_OFF_CAP / TABLE_OFF_ARRAY_BUF / ARRAY_ELEM_SIZE / ARRAY_ELEM_OFF_VALUE | `src/codegen/emit.rs` + `tagged.rs` | layout constants |

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**:
  `param_kinds_for_arity(argc)` 小 refactor で ADR 0110 の
  position-stable 契約を arity-aware に拡張。
  `lower_namespace_builtin_call` の special-case 復活なし.
  Builtin::TableInsert 1 variant 追加で済む.
- [x] **#2 TDD (Codex critical)**: 12 e2e — happy 5 (append /
  middle / head / tail-pos / empty) + value-kind 2 (String /
  TaggedValue) + runtime trap 2 (pos=0 / pos>#t+1) + HIR
  negative 2 (non-Table arg0 / non-Number pos arity-3) +
  shadowing 1. 既存 1141 stay green.
- [x] **#3 FP**: `param_kinds_for_arity` は pure static data;
  `emit_table_insert_runtime` は effectful chokepoint helper.
- [x] **#4 CA (Codex critical)**: `src/cli/`, `src/pipeline.rs`,
  `src/parser/`, `src/lexer/`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: pos runtime trap (Lua spec compliance).
  非 Number pos (arity 3) は HIR reject (param_kinds_for_arity).
  Value any-kind: Lua spec 通り.
- [x] **#6 Documentation**: ADR 0111 doc + tagged-semantics §8 row
  + AGENTS.md `‣ 2.7r-stdlib-table` row extended.

## Test count delta

```
Step 0:  1141 → 1141 corpus +12 new (Day 0 全 Red except 1
                                      shadowing が fall-through で Green)
Step 1:  1141 → 1141 (HIR refactor + Builtin::TableInsert; 既存
                       1141 stay green; new tests Red at codegen
                       non-exhaustive)
Step 2:  1141 → 1153 (codegen: memmove extern + diagnostic global
                       + emit_table_insert_runtime helper + emit
                       arm + ADR 0110 check fix for expected
                       TaggedValue skip; 11 Red → Green)
Step 3:  1141 → 1153 (clippy + fmt)
Step 4:  1141 → 1153 (docs only)

Final: 1141 → 1153 green, single atomic commit
  feat(hir,codegen,docs): table.insert + arity-sensitive param_kinds (ADR 0111)
```

## Verification

- `cargo test --no-fail-fast` → **1141 → 1153**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/ src/codegen/tagged.rs` → **0**
- Manual smoke:
  ```bash
  echo 'local t = {1, 2, 3}
  table.insert(t, 4)
  table.insert(t, 1, 0)
  table.insert(t, 3, 99)
  for i = 1, #t do print(t[i]) end' > /tmp/i.lua
  cargo run --quiet -- compile /tmp/i.lua && /tmp/i
  # Expected: 0 / 1 / 99 / 2 / 3 / 4
  ```

## Future work

- **`table.remove(t)` / `(t, pos)`** — mutation pair (ADR 0114 候補).
- **String ABI refactor (ptr+len)** — **ADR 0112** (直後).
- **`string.char(...)` proper** — ADR 0113 (ABI refactor 後).
- **malloc OOM consolidation** — ADR 0112 bundle 予定.
- **`table.unpack / pack / sort / move`** — incremental.
- **TaggedValue arg runtime tag-check** (ADR 0110 deferred).
- **Non-integral pos trap** — arg-validation policy ADR.
- **`s:insert(v)` method syntax** — Phase 3 metatables.

## ADR number / phase tag

ADR 0111 = `table.insert` + arity-sensitive param_kinds refactor.
Phase tag: `2.7r-stdlib-table` (continues ADR 0106/0107/0108
sub-lane; AGENTS.md row extended).
