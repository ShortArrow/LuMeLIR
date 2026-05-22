# 0113. Phase 2.7v-stdlib-string-char: `string.char(...)` + NaN/Inf/integer gate

- **Status:** Accepted
- **Date:** 2026-05-22
- **Deciders:** ShortArrow

## Replan provenance

ADR 0112 (`875e9cc`, 2026-05-19) で boxed string object ABI
`{i64 len, i8 data[len+1]}` が main にランディング、1167/1167
green、embedded-NUL semantics 動作確認済。0112 doc 末尾で
「ADR 0113 (string.char proper) — first new producer enabled by
this ABI」と明記、本 ADR でこの bridge を渡す。

Codex post-0112 6-視点 review verdict: **A (`string.char`) =
Refactor → Go**、**B (`string.byte` multi-byte bundle) = No-Go**。

3-ADR sequence は完結:
- ADR 0111 = `table.insert` (ABI-independent feature bridge)
- ADR 0112 = String ABI refactor (boxed object + OOM)
- **ADR 0113 = `string.char` proper** (本 ADR)

Codex critical (6-視点) fixes baked in:

1. **NaN/Inf/integer gate BEFORE `emit_f2i`** — ADR 0105 / 0109
   carry-over (`emit_f2i` は raw `arith.fptosi` で UB:
   `src/codegen/emit.rs:8812-8824`、11 caller のうち 4 sites のみ
   NaN guard 済) をここで回収。`string.char(1.5)` / `0/0` /
   `1/0` の Lua spec 違反 trap を必須化。
2. **Variadic Number arg-kind spec** — 既存
   `param_kinds_for_arity(argc) -> &'static [ValueKind]` API は
   variadic で argc 数だけ Number を要求できない。`string.char`
   は最初の trigger; 新 method `expected_param_kind(argc, pos)`
   で per-position 関数化、既存 builtin は内部 fallback。
3. **New lane `2.7v-stdlib-string-char`** — `2.7q` row は
   0103/0104/0105/0109 で既に肥大、0112 と同様に独立 row 分離。
4. **`primitive.rs` に Lua-spec policy を入れない** — codex
   critical CA: integer/range rule は `emit.rs` に閉じ込め。
5. **`string.byte` multi-byte form は bundle しない** —
   multi-result builtin policy が ADR 主題をぼかす (codex B
   No-Go); 0109 future-work に残置済。

```lua
-- 動くようになる:
string.char(65, 66, 67)           -- "ABC"
#string.char()                    -- 0 (empty)
#string.char(0, 65, 0, 66)        -- 4 (embedded NUL, ADR 0112 ABI payoff)
string.byte(string.char(0,65,0,66), 1)  -- 0 (NUL roundtrip)
string.char(255)                  -- "\xff"

-- Trap surface:
string.char(256)                  -- s_string_char_out_of_range
string.char(-1)                   -- s_string_char_out_of_range
string.char(1.5)                  -- s_string_char_non_integer
string.char(0/0)                  -- s_string_char_out_of_range (NaN fails Ord)
string.char(1/0)                  -- s_string_char_out_of_range (Inf > 255)
```

## Non-goals

- **`string.byte(s, i, j)` multi-byte form** — multi-result
  builtin policy。独立 ADR 候補。
- **`string.format` / `reverse` / `find` / `match` / `gmatch`** —
  scope explosion。
- **`emit_f2i` NaN/Inf gate 全方位 sweep** — `string.char` 1 site
  のみ。他の non-NaN-guard caller は別 ADR で Tidy First trigger
  待ち。
- **OOM consolidation 全方位** — ADR 0112 で scope 外。
- **`primitive.rs` に integer/range validation helper** — Lua-spec
  policy は emit.rs (codex critical CA)。
- **MLIR shape pin tests** — e2e で機能 verify。
- **`pcall` / `error` 値伝播** — 現状 trap は `exit(1)`。

## Lua 5.4 §6.4 — deviations

- **Spec準拠**: byte sequence return; arity 0+; integer-valued
  Number in [0, 255] per arg; embedded NUL full support (ADR
  0112 ABI payoff)。
- **Deviation**: spec で out-of-range / non-integer は
  "bad argument" error (`luaL_error`)。本実装は
  `emit_exit_with_message` (exit(1) + printf) で trap
  (`error()` builtin と同 surface)。Future `pcall` 対応 ADR で
  再考可。
- **Strict integer check**: `x == libm_floor(x)` で 1.0 OK /
  1.5 reject。

## 設計

### 1. HIR (`src/hir/ir.rs`)

```rust
enum Builtin {
    // ... existing variants ...
    StringChar,  // NEW (variadic Number → String)
}

impl Builtin {
    pub fn string_from_method(method: &str) -> Option<Builtin> {
        match method {
            // ... existing arms ...
            "char" => Some(Builtin::StringChar),  // NEW
            _ => None,
        }
    }

    pub fn arity(self) -> (usize, usize) {
        match self {
            // Print precedent: variadic
            Builtin::StringChar => (0, usize::MAX),
            // ...
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::StringChar => "string.char",
            // ...
        }
    }

    pub fn ret_kinds(self) -> &'static [ValueKind] {
        // String-returning or-pattern に StringChar を追加
    }

    /// NEW (codex critical #3): variadic / position-dependent
    /// per-position kind spec。`param_kinds_for_arity` の
    /// `&'static [ValueKind]` API では string.char の variadic
    /// Number (argc 個の Number) を表せないため per-position
    /// 関数化。既存 builtin は param_kinds_for_arity 経由 —
    /// zero-regression。
    pub fn expected_param_kind(self, argc: usize, pos: usize) -> Option<ValueKind> {
        match self {
            Builtin::StringChar => Some(ValueKind::Number),
            _ => self.param_kinds_for_arity(argc).get(pos).copied(),
        }
    }
}
```

### 2. HIR (`src/hir/mod.rs`)

- `infer_kind` の String or-pattern に `StringChar` を追加 (1 LOC)。
- `lower_namespace_builtin_call` の check loop driver を
  `param_kinds_for_arity(argc).get(i)` から
  `expected_param_kind(argc, i)` に swap。ADR 0110 の TaggedValue
  sentinel logic (expected==TaggedValue or actual==TaggedValue で
  skip) は保持。Same-semantics swap — zero-regression。

### 3. Codegen diagnostic globals

`emit_string_global` (boxed object form per ADR 0112) で:

```
s_string_char_out_of_range = "bad argument to 'char' (value out of range)"
s_string_char_non_integer  = "bad argument to 'char' (number has no integer representation)"
```

文言は Lua reference impl error message に準拠。

### 4. `emit_trap_if` helper (新規 generic)

`emit_table_index_nan_trap_if` (ADR 0086, `src/codegen/emit.rs:598-618`)
の generic 化 — `msg_global: &str` を取って任意 diagnostic に
切り替え。`emit_check_byte_arg` の 2 trap branch で reuse。

```rust
fn emit_trap_if(context, block, cond_i1, msg_global, types, loc) {
    // scf.if (cond_i1) {
    //   exit_with_message(emit_addressof(msg_global))
    // } else { /* noop */ }
}
```

`emit_table_index_nan_trap_if` 自体は ADR 0086 で `s_table_index_nan`
hardcoded で導入されているため `emit_trap_if(cond, "s_table_index_nan")`
への置き換えは Tidy First trigger 待ち (本 ADR scope 外、別 ADR
で sweep 候補)。

### 5. `emit_check_byte_arg` (新規 chokepoint)

```rust
fn emit_check_byte_arg(arg_f64) -> i64 {
    // Step 1: range check 0.0 <= x <= 255.0
    //   - cmpf Oge / Ole は NaN で false (unordered)
    //   - +Inf は Ole 失敗 → trap
    //   - NaN / Inf は自然に reject される
    let in_range = (x >= 0.0) AND (x <= 255.0);
    emit_trap_if(NOT in_range, "s_string_char_out_of_range");

    // Step 2: integer check x == floor(x)
    //   - range pass 後の x は有限 [0, 255] → libm floor 安全
    //   - cmpf Oeq で strict equality (1.0 OK, 1.5 reject)
    let floored = libm floor(x);
    let is_int = (x == floored);
    emit_trap_if(NOT is_int, "s_string_char_non_integer");

    // Step 3: f2i (safe — validated finite integer in [0, 255])
    emit_f2i(x)
}
```

設計判断:
- **Order**: Range FIRST → Integer SECOND → f2i LAST。
  Range pass で x が有限保証 → libm floor 安全 → fptosi 安全。
- **`emit_libm_call("floor", ...)`** は ADR 0101 で declared、
  `FloorDiv` (`:8680`) / `emit_lua_mod` (`:10733`) で reuse 実績
  あり — 新規 extern 宣言不要。
- **2 diagnostic globals** — out-of-range vs non-integer で
  Lua reference impl の異なる error wording に一致。

### 6. `emit_string_char_runtime` (新規 chokepoint)

```rust
fn emit_string_char_runtime(args_f64: &[Value]) -> ptr {
    // ADR 0112: boxed object alloc with len = args.len() (static).
    let len_i64 = const_i64(args_f64.len() as i64);
    let new_obj = emit_string_obj_alloc(len_i64);
    let data = emit_string_obj_data(new_obj);

    // Per-arg validate + store i8.
    for (i, arg_f64) in args_f64 {
        let byte_i64 = emit_check_byte_arg(arg_f64);
        let byte_i8 = arith::trunci(byte_i64, types.i8);
        let dst = emit_byte_offset_ptr_dynamic(data, i);
        emit_store(byte_i8, dst);
    }

    // ADR 0112: finalize compat NUL terminator.
    emit_string_obj_finalize_nul(new_obj, len_i64);
    new_obj
}
```

`len` は call-site static (Rust `args.len()`) — runtime mul なし、
alloc-size overflow なし。

### 7. Builtin emit arm

`Callee::Builtin(Builtin::StringChar)` arm を `StringRep` / `TableConcat`
arm の隣に追加。args を全て f64 lower → `emit_string_char_runtime`
へ delegate。

## Reuse (file:line citations)

| Helper | Path | Purpose |
|---|---|---|
| `emit_string_obj_alloc` (ADR 0112) | `src/codegen/primitive.rs:486` | object alloc + len store |
| `emit_string_obj_data` (ADR 0112) | `src/codegen/primitive.rs:468` | data ptr (gep+8) |
| `emit_string_obj_finalize_nul` (ADR 0112) | `src/codegen/primitive.rs:516` | compat NUL terminator |
| `emit_byte_offset_ptr_dynamic` | `src/codegen/primitive.rs` | gep i8 dynamic offset |
| `emit_store` | `src/codegen/primitive.rs` | i8 byte store |
| `emit_libm_call` ("floor") | `src/codegen/emit.rs:8680` (FloorDiv site) | integer check |
| `emit_f2i` | `src/codegen/emit.rs:8812-8824` | f64 → i64 (post-gate) |
| `emit_exit_with_message` (ADR 0112) | `src/codegen/primitive.rs:187-192` | trap (exit 1 + println) |
| `emit_addressof` | `src/codegen/primitive.rs` | diagnostic global ptr |
| `emit_string_global` (ADR 0112) | `src/codegen/emit.rs:532-538` | diagnostic global emit |
| `Builtin::from_namespace_method` (ADR 0103) | `src/hir/ir.rs:488` | call-site dispatch |
| `Builtin::param_kinds_for_arity` (ADR 0111) | `src/hir/ir.rs:662-708` | base behavior for non-StringChar |
| ADR 0110 TaggedValue sentinel skip | `src/hir/mod.rs` (check loop) | TaggedValue compat |
| `emit_table_index_nan_trap_if` shape | `src/codegen/emit.rs:598-618` | reference for `emit_trap_if` generic |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: 0112 直接 payoff。
  `emit_string_obj_*` helpers が "write N bytes + finalize NUL"
  producer pattern を想定通り再利用。`emit.rs` に閉じ込め。
- [x] **#2 TDD**: 14 e2e — happy (5) + arity 0 + embedded NUL
  (2 in `phase2_7u_string_abi.rs`) + range trap (2: 256/-1) +
  integer trap (3: 1.5/NaN/Inf) + HIR arg-kind negative +
  shadowing positive pin。
- [x] **#3 FP**: `expected_param_kind` API extension で variadic
  Number 表現。既存 `param_kinds_for_arity` は内部 fallback。
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/primitive.rs`, `src/codegen/tagged.rs`
  **zero-diff**。HIR 1 variant + 数行の dispatcher 拡張。
- [x] **#5 Security**: 4 hardening を 1 chokepoint
  (`emit_check_byte_arg`) に集約。Range FIRST で NaN/Inf
  natural reject、integer check 後 f2i 安全。args count × byte
  overflow なし (static len)。OOM は 0112 chokepoint 済。
- [x] **#6 Documentation**: NEW lane `‣ 2.7v-stdlib-string-char`
  (2.7q 拡張せず新 row、0112 precedent)。ADR 先頭に Lua §6.4
  deviations 明示。

## Test corpus delta

- `tests/phase2_stdlib_string.rs`: 12 new e2e (~150 LOC)
  - `string_char_basic_three` (happy)
  - `string_char_single_byte_pin` (happy)
  - `string_char_empty_arity_zero` (arity 0)
  - `string_char_byte_zero_low_edge` (range edge 0)
  - `string_char_byte_255_high_edge` (range edge 255)
  - `string_char_byte_256_traps` (range trap, codex critical)
  - `string_char_byte_negative_traps` (range trap)
  - `string_char_non_integer_traps` (integer trap 1.5)
  - `string_char_nan_traps` (NaN trap)
  - `string_char_inf_traps` (Inf trap)
  - `string_char_rejects_non_number` (HIR BuiltinArgKindMismatch)
  - `string_char_shadowed_respects_user_table` (codex critical)
- `tests/phase2_7u_string_abi.rs`: 2 new e2e (~30 LOC)
  - `string_abi_char_produces_embedded_nul` (NUL producer × 0112 ABI)
  - `string_abi_char_nul_roundtrip_via_byte` (producer × consumer
    roundtrip)

**Final: 1167 → 1181 green (+14)。**

## Risks

| Risk | Mitigation |
|---|---|
| NaN/Inf UB in f2i (codex critical #5) | gate FIRST: range check (cmpf Ord*) で NaN/Inf reject、integer check は finite x で libm floor 安全、f2i は validated x のみ。 |
| variadic Number に static slice API が窮屈 | 新 method `expected_param_kind(argc, pos)` で per-position 関数化、既存 `param_kinds_for_arity` は内部 fallback。zero-regression。 |
| ADR 0110 check loop 互換性 | check loop driver swap は same semantics、TaggedValue sentinel logic 変更なし。 |
| 既存 1167 tests regression | full corpus run で確認: 1167 stay green + 14 new = 1181。 |
| `libm floor` rounding | range check 後の x は有限 [0, 255]、IEEE-754 round-to-nearest-int 仕様通り。 |
| `cmpf Oge/Ole` NaN 動作 | Ord* は NaN で false → range trap → intended。 |
| trap message global 重複 | `s_string_char_*` 新規 2 件、既存 name collision なし (grep 確認)。 |

## Future work

- **ADR 0114 候補**: `string.byte(s, i, j)` multi-byte form
  (multi-result builtin policy + 0109 future-work 回収)。
- **ADR 0115 候補**: `emit_f2i` NaN/Inf gate 全方位 sweep
  (StringByte:7893 / StringSub:8037/8052 / TableConcat:8235/8252
  / TableInsert:8331 / StringRep:9226 / Bitwise:8687/8688/8780)。
- **ADR 0116 候補**: `string.format` (printf-like)。
- **ADR 候補**: `string.reverse` / `find` / `match` / `gmatch`。
- **`pcall` / `error` 値伝播** — Lua spec "bad argument" を proper
  `error()` 経由に変える ADR (cross-cutting)。
- **`emit_table_index_nan_trap_if` → `emit_trap_if` migration**
  (ADR 0086 hardcoded site の Tidy First refactor)。

## Phase tag

`2.7v-stdlib-string-char` (新 row; 2.7q-stdlib-string は維持さ
れるが 0113 は別 lane — 0112 precedent と同様)。
