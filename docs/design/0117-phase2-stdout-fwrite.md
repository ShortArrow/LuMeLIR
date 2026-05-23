# 0117. Phase 2.devinfra-stdout-fwrite: `emit_print_string_obj` printf → fwrite chokepoint swap

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-22 (commit `2f674a2`)
- **Deciders:** ShortArrow

## Replan provenance

ADR 0116 (`io.write`) surfaced a pre-existing ADR 0112
carry-over: `emit_print_string_obj` used
`printf("%.*s", len, data)`, which per POSIX `%s` semantics
**stops at the first NUL byte** regardless of the precision.
Every stdout path (`print` Bool/Nil/String, `io.write`,
`emit_print_literal`, tagged-print Bool/String/Nil/Function/
Table, and diagnostic `emit_exit_with_message`) silently
truncated Lua strings containing embedded NULs — even though
ADR 0112's boxed ABI preserved the bytes (length / byte
readback / equality / hashing all worked).

Codex post-0116 6-視点 verdict **N (A 単独) = Best Go**
(over K = A + `io.read` bundle / L = A + `table.remove` bundle
— scope contamination).

## Codex critical fixes baked in

1. **Single chokepoint swap** — `emit_print_string_obj` from
   `printf("%.*s", len, data)` to `fwrite(data, 1, len,
   stdout)`. 9 callers all benefit automatically.
2. **`stdout` extern global** — first extern global in the
   codebase. `GlobalOperationBuilder`'s typed slot system
   rejects "no value, no constant"; the decl uses raw
   `OperationBuilder` to construct `llvm.mlir.global external
   @stdout() : !llvm.ptr` directly.
3. **`emit_println_string_obj` split** — two calls to
   `emit_print_string_obj` (the value, then `s_newline`) so
   both reuse the binary-safe chokepoint.
4. **Dead-global cleanup** — `fmt_str_lensafe` (`"%.*s\n"`) and
   `fmt_str_raw_lensafe` (`"%.*s"`) deleted. They were single-use
   format strings for the old printf path.
5. **No `io.read` bundle** — interactive resource policy is a
   separate ADR.
6. **No `table.remove` bundle** — feature lane vs Tidy First mix.

## Non-goals (top-of-ADR)

- **`io.read("*l")`** — separate ADR (0119); interactive harness.
- **`table.remove`** — feature lane (ADR 0118).
- **macOS / Windows libc** — macOS `__stdoutp` macro, Windows
  `_iob_func()` — Linux x86_64 / glibc only for this ADR.
- **`io.flush` + `fwrite` return value** — separate small ADR;
  short-write retry / disk-full handling out of scope.
- **OOM consolidation 全方位** — codex Q5 scope-drift guard from
  ADR 0112 maintained.

## Goals

1. Replace `printf("%.*s", len, data)` chokepoint with
   `fwrite(data, 1, len, stdout)` to restore Lua §2.4 "8-bit
   clean" stdout semantics.
2. Declare `stdout` extern global + `fwrite` extern function.
3. Delete `fmt_str_lensafe` + `fmt_str_raw_lensafe` (dead after
   swap).
4. Test corpus: 1229 → 1235 (+6 net = -1 inverted + 7 added).

## Lua 5.4 §2.4 compliance

- "Lua is also 8-bit clean: strings can contain any 8-bit value,
  including embedded zeros (`'\0'`)."
- Pre-0117: stdout truncates at NUL (spec violation).
- Post-0117: binary-safe stdout (spec compliant).

## 設計

### `fwrite` extern (`src/codegen/emit.rs`)

```rust
fn emit_fwrite_decl(context, module, types, loc) {
    let fwrite_fn_type = llvm::r#type::function(
        types.i64,  // ssize_t return
        &[types.ptr, types.i64, types.i64, types.ptr],
        false,
    );
    LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "fwrite"))
        .function_type(TypeAttribute::new(fwrite_fn_type))
        .linkage(External)
        .build()
}
```

Mirrors `emit_printf_decl` shape.

### `stdout` extern global

```rust
fn emit_stdout_extern_decl(context, module, types, loc) {
    OperationBuilder::new("llvm.mlir.global", loc)
        .add_regions([Region::new()])
        .add_attributes(&[
            (Identifier::new(context, "global_type"),
             TypeAttribute::new(types.ptr).into()),
            (Identifier::new(context, "sym_name"),
             StringAttribute::new(context, "stdout").into()),
            (Identifier::new(context, "linkage"),
             llvm::attributes::linkage(context, External)),
        ])
        .build()
}
```

Equivalent MLIR text: `llvm.mlir.global external @stdout() :
!llvm.ptr`. The typed `GlobalOperationBuilder` API requires
all slots Set (including `value` + `initializer`); external
globals omit both, so the raw `OperationBuilder` builds the op
directly.

### `emit_print_string_obj` swap (`src/codegen/primitive.rs`)

```rust
pub(crate) fn emit_print_string_obj(context, block, s_ptr, types, loc) {
    let len = emit_string_obj_len(block, s_ptr, types, loc);
    let data = emit_string_obj_data(context, block, s_ptr, types, loc);
    let one_i64 = const_i64(1);
    let stdout_addr = emit_addressof(context, block, "stdout", types, loc);
    let stdout_ptr = emit_load(block, stdout_addr, types.ptr, loc);
    // fwrite(data, 1, len, stdout) — binary-safe.
    let call_op = OperationBuilder::new("llvm.call", loc)
        .add_operands(&[data, one_i64, len, stdout_ptr])
        .add_attributes(/* callee=fwrite, segmentSizes=[4,0], ... */)
        .add_results(&[types.i64])
        .build();
    block.append_operation(call_op);
}
```

Return value (short-write count) ignored — matches Lua spec
where `io.write` does not surface short-write errors.

### `emit_println_string_obj` split

```rust
pub(crate) fn emit_println_string_obj(context, block, s_ptr, types, loc) {
    emit_print_string_obj(context, block, s_ptr, types, loc);
    let newline_ptr = emit_addressof(context, block, "s_newline", types, loc);
    emit_print_string_obj(context, block, newline_ptr, types, loc);
}
```

`s_newline` is a boxed-object form `"\n"` (1 byte data, len=1).
Two fwrite calls compose; glibc stdout buffering coalesces them.

`primitive.rs` cannot depend on `emit.rs`'s `emit_print_literal`
wrapper, so the s_newline emission is inlined.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_printf_decl` (precedent) | `src/codegen/emit.rs:749-766` | extern function decl pattern |
| `GlobalOperationBuilder` (typed; rejected for extern) | melior 0.27 | required Set slots → use raw OperationBuilder |
| `OperationBuilder` (raw) | melior 0.27 | extern global without value/initializer |
| `Linkage::External` | melior 0.27 | external symbol |
| `emit_string_obj_len` / `_data` (ADR 0112) | `src/codegen/primitive.rs:457-475` | header read |
| `emit_addressof` + `emit_load` | `src/codegen/primitive.rs` | FILE * dereference |
| `s_newline` global (ADR 0112) | `src/codegen/emit.rs:235` | "\n" boxed object |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: ADR 0112 promise repair.
  Single chokepoint swap; 9 callers all benefit. New extern
  global pattern (first in codebase) using raw OperationBuilder
  workaround for `GlobalOperationBuilder`'s typed slot system.
- [x] **#2 TDD**: 6 net new e2e (1 inverted carry-over pin + 7
  new), 2 files (`tests/phase2_stdlib_io.rs` + new
  `tests/phase2_devinfra_stdout_fwrite.rs`).
- [x] **#3 FP**: pure ASCII path unchanged in user-visible
  behavior; the effectful boundary is the runtime libc call.
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/hir/`, `src/codegen/tagged.rs` **zero-diff**.
- [x] **#5 Security / integrity**: output-integrity bug repaired
  (data loss → binary-safe). Lua §2.4 / §6.6 compliance restored.
- [x] **#6 Documentation**: phase tag `2.devinfra-stdout-fwrite`
  (cross-cutting Tidy First; ADR 0114 `2.7w` precedent). ADR
  0112 doc receives a `RESOLVED by ADR 0117` annotation on the
  stdout NUL truncation carry-over.

## Test count delta

1229 → 1235 (+6 net = +7 new − 1 inverted).

`tests/phase2_stdlib_io.rs` (4 changes):
- `io_write_embedded_nul_loses_bytes_after_nul` → INVERTED to
  `io_write_embedded_nul_preserves_bytes`
- NEW: `io_write_middle_nul_preserves_bytes`
- NEW: `io_write_lone_nul_preserves_bytes`
- NEW: `io_write_ascii_regression` (sanity)

`tests/phase2_devinfra_stdout_fwrite.rs` NEW (3 tests):
- `print_embedded_nul_preserves_bytes_with_newline`
- `print_middle_nul_preserves_bytes_with_newline`
- `print_ascii_regression_with_newline`

## Critical files

- `src/codegen/emit.rs`:
  - `emit_fwrite_decl` (~25 LOC)
  - `emit_stdout_extern_decl` (~30 LOC, raw OperationBuilder)
  - Module init wiring (~5 LOC)
  - Delete `fmt_str_lensafe` + `fmt_str_raw_lensafe` (-15 LOC)
- `src/codegen/primitive.rs`:
  - `emit_print_string_obj` rewrite (printf → fwrite, ~50 LOC delta)
  - `emit_println_string_obj` split (~12 LOC)
- `tests/phase2_stdlib_io.rs` (~60 LOC delta)
- `tests/phase2_devinfra_stdout_fwrite.rs` NEW (~75 LOC)
- `docs/design/0112-phase2-string-abi-refactor.md` (~6 LOC
  supersede note on NUL truncation)

**Zero-diff (CA invariant)**:
`src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`,
`src/hir/`, `src/codegen/tagged.rs`.

## Risks

| Risk | Mitigation |
|---|---|
| melior 0.27 `GlobalOperationBuilder` rejects external (no value, no constant) | Raw `OperationBuilder` construct `llvm.mlir.global external @stdout() : !llvm.ptr` directly |
| glibc `stdout` exported under an alias (`__GI___stdoutp` etc.) | x86_64 glibc exports plain `stdout` symbol (`nm -D` confirmed); ELF resolver aliases |
| `fwrite` short-write (disk full / pipe close) silently ignored | Lua spec also ignores; future `io.flush` ADR can add return-value handling |
| 2 fwrite calls per println adds buffering overhead | glibc line/block buffering coalesces; negligible |
| Diagnostic global with embedded NUL | None exist today; future trap messages must stay NUL-free |
| Dead `fmt_str_lensafe` / `fmt_str_raw_lensafe` deletion breaks unknown callers | grep-verified single-use before deletion |
| macOS / Windows portability | Out of scope; future ADR can wrap stdout access in a platform shim |

## Future work

- **ADR ?? — `io.flush` + `fwrite` return value check**: buffer
  control + short-write surfacing.
- **macOS / Windows libc cross-platform**: `__stdoutp` /
  `_iob_func()` shims.
- **OOM consolidation 全方位**: deferred from ADR 0112.

## Phase tag

`2.devinfra-stdout-fwrite` (cross-cutting Tidy First lane; ADR
0114 `2.7w-emit-f2i-gate-sweep` precedent — codegen-internal
correctness hardening, no HIR / parser surface change).
