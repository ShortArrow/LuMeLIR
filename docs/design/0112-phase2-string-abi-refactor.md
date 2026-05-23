# 0112. Phase 2.7u-string-abi-refactor: Boxed string object + string-alloc OOM consolidation

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-19
- **Deciders:** ShortArrow
- **Supersedes (ABI surface):** ADR 0024 (`!llvm.ptr` to NUL-term C-string)

## Replan provenance

ADR 0111 (`table.insert`) は ABI-independent feature として
ADR 0112 (本 ADR) と ADR 0113 (`string.char` proper) への bridge
だった。Codex 6-視点 review が `string.char` を NoGo にしたのは、
ADR 0024 の C-string ABI が embedded-NUL を扱えない (`strlen`-
based length truncate、`strcmp`-based eq の false collision、
printf `%s` の NUL 切り) ためで、`string.char(0)` 等の spec-
compliant byte producer を入れる前に ABI 自体を直す必要があった。

Codex post-0111 6-視点 review (ADR 0112 scope) verdict:
**Refactor → Go (big-bang)**、critical 5 件:
- Q1 (b): **boxed string object** `{i64 len, i8 data[len+1]}`
  (NOT thin ptr+len pair — pair は tagged slot payload 8-byte
  制約に衝突).
- Q2 (i): tagged slot 16-byte 不変、TAG_STRING payload は object
  ptr.
- Q3: literal は header-prefixed global.
- Q4 (β): 全 consumer を **1 atomic commit で migration** —
  phased (γ) は NoGo (半分 migrate は壊れた surface を残す).
- Q5: OOM consolidation は **string alloc sites のみ** に
  bundle (table/hash grow / closure cell は範囲外).

## Non-goals (top-of-ADR)

- **`(ptr, len)` thin pair value ABI** — Q1 (a) NoGo: local /
  param / ret / tagged payload (8-byte) 全部破壊する.
- **Tagged slot 24-byte 拡張** — Q2 (ii) NoGo: table / hash /
  local ABI 巻き込み過ぎ. 16-byte 維持.
- **Phased per-consumer migration** — Q4 (γ) NoGo. 半分 migrate
  は壊れた surface を残す.
- **table grow / hash grow / closure cell の OOM consolidation** —
  Q5: scope が ABI から alloc sweep にずれる. 0112 で touch する
  string alloc sites のみに bundle.
- **`sscanf` length-bounded parse for `tonumber`** — embedded
  NUL 以降の bytes は silently drop (Lua spec: tonumber は
  partial parse で nil 返却可). 現状 sscanf を維持し deviation
  明記; 将来 arg-validation policy ADR で再考.
- **MLIR shape pin tests** — e2e で機能 verify; shape tests は
  将来の internal regression 用 (今回 deferred).
- **String interning / GC** — Phase 3 territory.

## 目標

```lua
-- ADR 0112 で動くようになる:
#"\x00"                          -- 1 (was 0)
#"A\x00B"                        -- 3 (was 1)
"A\x00B" == "A\x00B"             -- true (was true by coincidence)
"A\x00B" == "A"                  -- false (was true — strcmp 0)
"A\x00B" < "A\x00C"              -- true (was false — strcmp 0)
string.byte("A\x00B", 2)         -- 0 (was trap/wrong)
string.sub("A\x00B", 1, 2)       -- 2-byte string (was 1-byte)
string.rep("\x00", 3)            -- length 3 (was 0)
table.concat({"A\x00", "B"})     -- length 3 (was 1)
local t = {}; t["A\x00B"] = 1; print(t["A\x00B"])   -- 1 (was broken)
```

## Lua 5.4 spec (§2.4 / §3.4.4 / §6.4)

- String は **byte sequence**, embedded NUL 含む任意 byte 列.
- `#s` returns byte length (not 文字数; UTF-8 awareness は別).
- `s == t` is byte-equal (length + content).
- `s < t` is byte-lexicographic.
- `string.byte(s, i)` reads byte at index i (1-based).
- `string.sub(s, i, j)` extracts byte range.
- hash-key equality is byte-equal (length + content).

## 設計

### 1. String object layout

```
offset 0: i64 len      // truth-source for byte count
offset 8: i8 data[len]  // raw bytes (may contain NUL)
offset 8+len: i8 0      // compat NUL terminator (legacy printf,
                        // sscanf — kept as safety belt even
                        // after all consumers migrate to len-
                        // aware accessors)
```

Total allocation size: `len + 9` bytes
(`STRING_OBJ_HEADER_SIZE (8) + len + 1 compat NUL`).

Globals (static literals + diagnostics) are emitted as i8 arrays
with the same byte layout: little-endian length prefix + data +
NUL. `unsafe { String::from_utf8_unchecked }` tunnels the raw
bytes through `StringAttribute::new(&str)`.

### 2. Helpers (`src/codegen/primitive.rs`)

| Helper | Purpose |
|---|---|
| `emit_string_obj_len(s_ptr) -> i64` | `load i64 @ s_ptr+0` |
| `emit_string_obj_data(s_ptr) -> i8*` | `gep s_ptr + STRING_OBJ_OFF_DATA` |
| `emit_string_obj_alloc(len) -> ptr` | `malloc(len+9)` via OOM checker + store len at +0 |
| `emit_string_obj_finalize_nul(s_ptr, len)` | store i8 0 at data+len |
| `emit_string_obj_from_bytes(src, len) -> ptr` | alloc + memcpy + finalize |
| `emit_string_obj_eq(a, b) -> i1` | `len_a == len_b && memcmp(data_a, data_b, len) == 0` |
| `emit_string_obj_compare(a, b) -> i32` | memcmp on min(len) + length-diff tiebreak (3-way like strcmp) |
| `emit_string_obj_hash(s) -> i64` | FNV-1a over `len` bytes from data |
| `emit_print_string_obj(s)` | `printf("%.*s", trunci(len, i32), data)` |
| `emit_println_string_obj(s)` | same with newline |
| `emit_alloc_with_oom_check(size, oom_global)` | `malloc(size)` + null-check + trap |

### 3. New globals

- `s_alloc_oom = "out of memory"` (object form) —
  `emit_alloc_with_oom_check` trap target.
- `fmt_str_lensafe = "%.*s\n\0"`, `fmt_str_raw_lensafe =
  "%.*s\0"` — printf-len-safe format strings (raw C-strings,
  consumed by printf as `const char *`).

### 4. `emit_string_global` split

Two emitter functions for the two distinct uses of i8 array
globals:

- `emit_cstr_global` — raw NUL-term C-string (printf format
  strings only: `fmt`, `fmt_str`, `fmt_raw`, `fmt_str_raw`,
  `fmt_tostring_g`, `fmt_tonumber_lf`, `fmt_str_lensafe`,
  `fmt_str_raw_lensafe`).
- `emit_string_global` — boxed object form (`[i64 len_le bytes,
  data, 0]`). Used for **every Lua-value** global — `s_true`,
  `s_false`, `s_nil`, `s_typename_*`, `s_tab`, `s_newline`, all
  diagnostic message strings (`s_assert_failed`,
  `s_table_oob`, ..., `s_alloc_oom`), and user literals via
  `collect_string_pool` (BTreeSet dedup unchanged).

### 5. Tagged slot ABI

TAG_STRING payload remains a ptr (8 bytes); the slot stays
16-byte. Only the **target of the ptr** changes from a NUL-term
i8 array to a boxed object header. All tagged.rs payload
read/write sites stay identical.

### 6. Consumer migration matrix

| # | Site | Migration |
|---|---|---|
| 1 | `#s` operator | strlen → `emit_string_obj_len` |
| 2 | `Builtin::StringLen` | strlen → `emit_string_obj_len` |
| 3 | `Builtin::StringByte` | strlen → header len; data ptr + offset |
| 4 | `Builtin::StringSub` | strlen → header len; `emit_string_slice` rewrites to `emit_string_obj_from_bytes(data + off, len)` |
| 5 | `Builtin::StringRep` | header len; alloc new object; copy from data |
| 6 | `Builtin::StringUpper` / `Lower` | header len + alloc object + memcpy + case loop + finalize NUL |
| 7 | `Builtin::TableConcat` (sep) | sep header len; memcpy from sep data; result is boxed object |
| 8 | dispatch_len/_str helpers | header len for both NUMBER (after snprintf wrap) and STRING arms |
| 9 | `..` concat | both operand lens + data ptrs; alloc combined object |
| 10 | `emit_concat` allocator | `emit_string_obj_alloc` + memcpy lhs.data, rhs.data |
| 11 | hash-key eq (`emit_hash_key_eq_dispatched`) | strcmp → `emit_string_obj_eq` |
| 12 | TaggedValue == TaggedValue String arm (`emit_tagged_eq_local_local`) | strcmp → `emit_string_obj_eq` |
| 13 | TaggedValue runtime eq String arm | strcmp → `emit_string_obj_eq` |
| 14 | `emit_string_cmp` (eq/ne + lt/le/gt/ge) | strcmp → `emit_string_obj_compare` (length-aware 3-way) |
| 15 | `emit_tostring(Number)` | snprintf → scratch; strlen on scratch (NUL-term); wrap via `emit_string_obj_from_bytes` |
| 16 | `emit_print_value_raw` String/Bool/Nil | printf `%s` → `emit_print_string_obj` |
| 17 | `emit_print_tagged_local` (Bool/String/Nil/Function/Table) | same |
| 18 | `emit_print_literal` (tab/newline) | same |
| 19 | `emit_exit_with_message` | printf `%s\n` → `emit_println_string_obj` |
| 20 | `emit_tonumber` String arm | sscanf reads `emit_string_obj_data(value)` (NOT object ptr) |

`emit_string_hash` (the old strlen-based FNV) is removed —
callers route through `emit_string_obj_hash`.

### 7. OOM consolidation (Q5 — scoped)

`emit_alloc_with_oom_check(size, "s_alloc_oom")` is the
chokepoint for every string-alloc site:

- `emit_string_obj_alloc` (used by concat, slice, rep, table-
  concat, upper/lower, empty string, snprintf-wrap)

Out of scope: table grow, hash grow, closure cell alloc, table
header alloc. Codex critical scope-drift guard.

### 8. Deviations

- **`sscanf` for `tonumber`**: receives data ptr. Embedded NUL
  silently truncates the parse (no spec violation — Lua allows
  partial-parse → nil; current path stays sscanf, future arg-
  validation ADR may revisit).
- **OOM trap untestable from Lua**: no language-visible OOM
  scenario, so no e2e. Helper exists for uniform alloc surface.
- **stdout NUL truncation (RESOLVED by ADR 0117)**: the original
  ADR 0112 left `emit_print_string_obj` on `printf("%.*s", len,
  data)`, which per POSIX `%s` semantics stops at the first NUL —
  defeating the boxed-ABI promise at the stdout chokepoint. ADR
  0117 swapped the chokepoint to `fwrite(data, 1, len, stdout)`,
  restoring binary-safe Lua §2.4 "8-bit clean" stdout.

## Non-ad-hoc framing

- Single ABI swap in 1 atomic commit (after WIP commits on
  feature branch `adr-0112-string-abi` are squashed). Phased
  consumer migration NoGo per Codex Q4.
- Boxed-object pattern matches Table / Function (heap-allocated
  with ptr carrier) — uniform Lua-value ABI.
- `emit_string_global` rewrite is the chokepoint: ~50 globals
  switch in one place.
- Length helpers (`emit_string_obj_len` / `_data`) make every
  consumer site mechanical.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_libc_call_ptr` (malloc) | `src/codegen/primitive.rs` | object alloc |
| `emit_libc_call_*` (memcpy/memmove/memcmp) | `src/codegen/primitive.rs` | data ops |
| `emit_load` / `emit_store` | `src/codegen/primitive.rs` | header r/w |
| `emit_byte_offset_ptr` / `_dynamic` | `src/codegen/primitive.rs` | gep |
| `emit_addressof` | `src/codegen/primitive.rs` | global ptr |
| `emit_exit_with_message` | `src/codegen/primitive.rs` | trap (migrated to use println string obj) |
| `collect_string_pool` | `src/codegen/emit.rs` | dedup unchanged |
| Tagged slot helpers | `src/codegen/tagged.rs` | payload unchanged |
| `scf::r#if` / `scf::r#while` | melior 0.27 | OOM trap, FNV loop |

## Test corpus delta

- `tests/phase2_7u_string_abi.rs` — 14 new e2e (`#`, eq, lex,
  byte, sub, rep, concat, table.concat, upper, hash key,
  string.len) all length-aware on embedded NUL.
- Existing 1153 tests stay green (NUL-free byte sequences are
  represented identically in the new object form, so behaviour
  is observationally unchanged for them).

**Final: 1153 → 1167 green.**

## Risks

| Risk | Mitigation |
|---|---|
| Codex critical: 半分 migrate で broken state を残す | atomic squash to main. WIP commits on feature branch are dev safety net; main lands one commit. |
| Existing 1153 regression | NUL-free strings encode identically; eq/compare/hash give same byte-level result; print visible output unchanged. |
| melior 0.27 struct global init complexity | i8 array fallback with LE-encoded len + data + NUL (used). `String::from_utf8_unchecked` tunnels raw bytes through `StringAttribute::new`. |
| `printf("%.*s", len_i32, data)` length cast | `arith.trunci i64 → i32`; Lua strings practically far below `i32::MAX`. |
| OOM trap untestable from Lua | accepted; helper exists for uniform alloc surface. |
| 1 commit で 700+ LOC 変更 — review cost | ADR doc lists every migrated site; matrix above is canonical. |

## Future work

- **ADR 0113 = `string.char(...)` proper** — direct payoff.
  **RESOLVED by ADR 0113 (2026-05-22)**.
- **stdout NUL truncation** (`emit_print_string_obj` used
  `printf("%.*s")`, POSIX `%s` stops at NUL) — was not on this
  list at the time of ADR 0112 but discovered while landing
  ADR 0116. **RESOLVED by ADR 0117 (2026-05-22)** via fwrite
  chokepoint swap.
- **MLIR shape regression tests** for string object layout.
- **String interning** — Phase 3 GC.
- **UTF-8 awareness** — `utf8.*` library.
- **sscanf length-bounded parse for tonumber** — strtod or
  manual parser.
- **table grow / hash grow OOM consolidation** — separate Tidy
  First ADR.

## Phase tag

`2.7u-string-abi-refactor` (new cross-cutting infra lane,
similar to `2.7t arg-kind validation` precedent).
