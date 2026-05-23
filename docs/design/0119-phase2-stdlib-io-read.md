# 0119. Phase 2.7x-stdlib-io: `io.read([format])` line input

- **Status:** Accepted
- **Date:** 2026-05-23 (commit `782dff0`)
- **Deciders:** ShortArrow

## Replan provenance

ADR 0118 (`table.remove`) completed table.* mutation surface
(concat/insert/remove). Codex post-0118 6-視点 verdict for the
next step was **A + C + E + getline = Strong Go** for
`io.read([format])`:

- **A**: Full `io.read([format])` arity (0, 1)
- **C**: HIR-time reject unsupported formats (only `"*l"` / `"l"`)
- **E**: `ret_kinds = [TaggedValue]` (nil-on-EOF)
- **getline**: POSIX dynamic-buffer reader (fgets / hand-rolled
  fgetc loop rejected)

`io.read` completes the io.* namespace write/read pair (ADR
0116 write + 0117 stdout fwrite + 0119 read).

## Codex critical fixes baked in

1. **`stdin` extern global** + **`getline` extern function** —
   mirror of ADR 0117's `stdout` / `fwrite` decl pattern.
2. **`emit_local_init_tagged` Builtin-TaggedValue arm reuse**
   (added by ADR 0118) — `local s = io.read(); ...` works
   immediately without further codegen plumbing.
3. **HIR-time format validation** — only literal `"*l"` / `"l"`
   accepted; other Str literals and non-literal expressions both
   `BuiltinArgKindMismatch`. MVP constraint; future ADR can lift
   the literal-only restriction.
4. **POSIX getline**, not fgets — fgets fixed-size buffer
   requires truncation handling or loop; getline grows the
   buffer dynamically.
5. **Boxed-string copy** — getline's malloc'd buffer is copied
   into a fresh ADR 0112 boxed string object, then freed. Lua-
   visible string lifetime is isolated from libc realloc.
6. **No `print(io.read())` direct support** — Print / io.write
   Builtin-TaggedValue source dispatch is a separate small
   ADR; workaround `local s = io.read(); print(s)`.

## Non-goals (top-of-ADR)

- **`io.read("*n")` / `"*a"`** — number / read-all formats;
  HIR-time reject for now, future ADRs each.
- **`io.read(n)` byte-count format** — Lua spec form; future
  ADR.
- **Multi-format `io.read("*l", "*n")`** — multi-result return
  framework still gated on a 2nd builtin trigger.
- **`io.open` / `io.close` / file handles** — Phase 3.
- **`io.flush` + getline / fwrite return value** — separate
  small ADR.
- **`print(io.read())` direct** — Builtin-TaggedValue source
  dispatch in Print/io.write arms; separate ADR.
- **Non-literal format strings** — `local f = "*l"; io.read(f)`
  HIR-rejected by MVP.
- **macOS / Windows libc** — ADR 0117 scope; Linux x86_64 only.
- **Resource policy / line-length cap** — getline grows
  unboundedly per Lua spec; OOM handling deferred to future
  resource policy ADR.

## Goals

1. `Builtin::IoRead` variant + `io_from_method("read")` arm +
   `arity (0, 1)` + name "io.read" + `ret_kinds =
   [TaggedValue]`.
2. `param_kinds_for_arity`: arity 1 → `[String]` (format arg);
   arity 0 → `&[]`.
3. HIR-time literal-format validation in
   `lower_namespace_builtin_call`.
4. Codegen: `stdin` extern global + `getline` extern function.
5. `emit_io_read_runtime` helper — getline call + strip
   trailing newline + boxed string wrap (or NIL on EOF) + free
   getline buffer.
6. `Callee::Builtin(IoRead)` emit arm — alloca tmp tagged slot,
   call runtime, return slot ptr.
7. Test corpus: 1248 → 1258 (+10).

## Lua 5.4 §6.6 compliance

- `io.read(format)`: read from default input file (stdin).
- `format = "*l"` / `"l"`: read a line without the newline,
  return String. EOF → nil.
- `format = "*n"` / `"*a"` / `n`: out of scope (MVP).
- `io.read()` (no arg): defaults to `"*l"`.

Spec deviation: non-literal format args HIR-rejected. Lua spec
permits dynamic format; future ADR can lift.

## 設計

### HIR

```rust
enum Builtin {
    IoRead,
}

impl Builtin {
    pub fn io_from_method(method: &str) -> Option<Self> {
        match method {
            "write" => Some(Builtin::IoWrite),
            "read" => Some(Builtin::IoRead),
            _ => None,
        }
    }

    pub fn arity(self) -> (usize, usize) {
        match self {
            Builtin::IoRead => (0, 1),
            // ...
        }
    }

    pub fn ret_kinds(self) -> &'static [ValueKind] {
        match self {
            Builtin::IoRead => &[ValueKind::TaggedValue],
            // ...
        }
    }

    pub fn param_kinds_for_arity(self, argc: usize) -> &'static [ValueKind] {
        match self {
            Builtin::IoRead => match argc {
                0 => &[],
                1 => &[ValueKind::String],
                _ => &[],
            },
            // ...
        }
    }
}
```

`infer_kind` for IoRead returns `ValueKind::TaggedValue`.

### HIR format validation (`lower_namespace_builtin_call`)

```rust
if matches!(builtin, Builtin::IoRead) && lowered_args.len() == 1 {
    match &lowered_args[0].kind {
        HirExprKind::Str(s) if s == "*l" || s == "l" => {}  // accepted
        HirExprKind::Str(s) => {
            return Err(HirError::BuiltinArgKindMismatch {
                builtin: "io.read".into(),
                arg_index: 1,
                expected: "\"*l\" or \"l\" (string literal)".into(),
                actual: format!("\"{}\"", s.escape_default()),
                offset: call_span.start,
            });
        }
        _ => {
            return Err(HirError::BuiltinArgKindMismatch {
                builtin: "io.read".into(),
                arg_index: 1,
                expected: "string literal \"*l\" or \"l\"".into(),
                actual: "non-literal expression".into(),
                offset: call_span.start,
            });
        }
    }
}
```

### `stdin` extern global + `getline` extern function

`stdin` decl mirrors `emit_stdout_extern_decl` (ADR 0117):

```rust
fn emit_stdin_extern_decl(context, module, types, loc) {
    OperationBuilder::new("llvm.mlir.global", loc)
        .add_regions([Region::new()])
        .add_attributes(&[
            (Identifier::new(context, "global_type"),
             TypeAttribute::new(types.ptr).into()),
            (Identifier::new(context, "sym_name"),
             StringAttribute::new(context, "stdin").into()),
            (Identifier::new(context, "linkage"),
             llvm::attributes::linkage(context, External)),
        ])
        .build()
}
```

`getline` decl mirrors `emit_fwrite_decl`:

```rust
fn emit_getline_decl(context, module, types, loc) {
    let ty = llvm::r#type::function(
        types.i64,
        &[types.ptr, types.ptr, types.ptr],
        false,
    );
    LLVMFuncOperationBuilder::new(context, loc)
        .body(Region::new())
        .sym_name(StringAttribute::new(context, "getline"))
        .function_type(TypeAttribute::new(ty))
        .linkage(External)
        .build()
}
```

### `emit_io_read_runtime`

```
Step 1: alloca char *lineptr = NULL; size_t n = 0
Step 2: load stdin FILE*
Step 3: n_read = getline(&lineptr, &n, stdin)   // i64

Step 4: scf::r#if (n_read == -1) EOF:
    out_slot.tag = TAG_NIL
    free(lineptr)   // POSIX free(NULL) is safe; non-NULL when
                    // getline allocated even on EOF
Step 4 else (non-EOF):
    buf = load lineptr
    last_byte = load (buf + n_read - 1, i8)
    has_nl = (last_byte == '\n')
    line_len = select(has_nl, n_read - 1, n_read)
    obj = emit_string_obj_alloc(line_len)   // ADR 0112
    memcpy(obj_data, buf, line_len)
    emit_string_obj_finalize_nul(obj, line_len)
    free(buf)   // libc buffer released; boxed copy is now standalone
    out_slot.tag = TAG_STRING
    out_slot.payload = obj (ptr stored as 8-byte slot)
```

### `Callee::Builtin(IoRead)` arm

```rust
Callee::Builtin(Builtin::IoRead) => {
    // Format arg is HIR-validated; codegen ignores it (arity 0
    // and arity 1 share the same runtime path).
    let out_slot = emit_alloca_slot_for_kind(
        context, block, ValueKind::TaggedValue, types, loc,
    );
    emit_io_read_runtime(context, block, out_slot, types, loc);
    Ok(out_slot)
}
```

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `Builtin::io_from_method` (ADR 0116) | `src/hir/ir.rs` | namespace dispatch extension |
| `emit_stdout_extern_decl` (ADR 0117) | `src/codegen/emit.rs:836` | mirror for stdin |
| `emit_fwrite_decl` (ADR 0117) | `src/codegen/emit.rs:804` | mirror for getline |
| `emit_addressof` + `emit_load` | `src/codegen/primitive.rs` | extern global access |
| `emit_string_obj_alloc` (ADR 0112) | `src/codegen/primitive.rs:486` | boxed string allocation |
| `emit_string_obj_data` (ADR 0112) | `src/codegen/primitive.rs:498` | data ptr |
| `emit_string_obj_finalize_nul` (ADR 0112) | `src/codegen/primitive.rs:543` | compat NUL |
| `emit_libc_call_ptr` ("memcpy") / `_void` ("free") | `src/codegen/primitive.rs` | line copy + buffer release |
| `emit_alloca_slot_for_kind` | `src/codegen/tagged.rs:163` | tmp tagged slot |
| `emit_local_init_tagged` Builtin arm (ADR 0118) | `src/codegen/emit.rs` | `local s = io.read()` path |
| `TAG_NIL` / `TAG_STRING` (ADR 0064) | `src/codegen/tagged.rs:44` | tag constants |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: io.* lane sibling
  extension; ADR 0117 stdout extern pattern symmetric; ADR 0118
  TaggedValue return pattern reused.
- [x] **#2 TDD**: 10 e2e — happy / default / `*l` / `l` / EOF
  nil / empty line / no-trailing-newline / multiple reads / 10KB
  long line / HIR negative `*n` / HIR negative non-literal.
- [x] **#3 FP**: pure HIR validation; effectful boundary is the
  runtime getline call.
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/primitive.rs`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: getline 行長無制限 (OOM risk) は Lua
  spec 通り; embedded NUL line は getline 自然対応; resource
  policy strict 化は別 ADR.
- [x] **#6 Documentation**: phase tag `2.7x-stdlib-io` (write +
  read pair; ADR 0116 と同 row 拡張).

## Test count delta

1248 → 1258 (+10) in `tests/phase2_stdlib_io.rs`.

| Test | Category | Stdin |
|---|---|---|
| `io_read_default_returns_line` | happy | `"hello\n"` |
| `io_read_star_l_format_works` | happy | `"hello\n"` |
| `io_read_l_format_works` | happy | `"hello\n"` |
| `io_read_eof_returns_nil` | EOF | `""` |
| `io_read_empty_line_returns_empty_string` | edge | `"\n"` |
| `io_read_no_trailing_newline` | edge | `"abc"` |
| `io_read_multiple_calls` | happy | `"a\nb\nc\n"` |
| `io_read_long_line` | long line | 10KB |
| `io_read_unsupported_format_rejects` | HIR negative | n/a |
| `io_read_non_literal_format_rejects` | HIR negative | n/a |

Pre-existing HIR static-typing limitation surfaces in
`empty_line` and `long_line` (TaggedValue source + `#` / `type`
mix rejected); tests use `io.write` byte-compare + delimiter
to verify the runtime dispatch instead — same pattern as ADR
0115 arg-table tests.

`run_with_stdin` harness (Stdio::piped() pattern) is added to
the same file (reuse of ADR 0115's pattern).

## Critical files

- `src/hir/ir.rs` (~15 LOC: variant + dispatch + arity + name +
  ret_kinds + param_kinds_for_arity)
- `src/hir/mod.rs` (~25 LOC: infer_kind + IoRead format
  validation in `lower_namespace_builtin_call`)
- `src/codegen/emit.rs`:
  - `emit_stdin_extern_decl` (~30 LOC)
  - `emit_getline_decl` (~25 LOC)
  - Module init wiring (~5 LOC)
  - `emit_io_read_runtime` helper (~210 LOC)
  - `Callee::Builtin(IoRead)` emit arm (~20 LOC)
- `tests/phase2_stdlib_io.rs` (~190 LOC: run_with_stdin helper + 10 e2e)

**Zero-diff (CA invariant)**:
`src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`,
`src/codegen/primitive.rs`, `src/codegen/tagged.rs`.

## Risks

| Risk | Mitigation |
|---|---|
| `getline` ABI return — -1 on EOF, >= 0 on success | i64 sentinel comparison; non-negative covers empty-line (1 byte = `'\n'`) and any positive content |
| `getline` buffer leak on EOF | Both branches `free(buf)` explicitly; POSIX `free(NULL)` no-op |
| Empty-line `"\n"` strip → 0-byte string | `arith::select(has_nl, n_read - 1, n_read)` → len 0 → empty boxed object (valid per ADR 0112) |
| Long-line OOM via getline realloc | Lua spec permits unbounded read; resource policy ADR later |
| `print(io.read())` direct fails | Non-goal; `local s = ...; print(s)` workaround |
| Non-literal format rejection too restrictive | Documented MVP; future ADR can lift |
| Format with embedded NUL | `s == "*l"` exact-byte compare → naturally rejected |
| Embedded NUL in input line | getline reads until `'\n'`; boxed ABI handles 8-bit clean since ADR 0117 |
| `lumelir run` itself uses stdin vs runtime io.read | `lumelir run -` reads stdin for the *compiler*; the generated binary's stdin is independent (separate process) |

## Future work

- **`io.read("*n")` / `"*a"`** — number / read-all formats.
- **`io.read(n)`** — byte-count format.
- **Multi-format `io.read(...)`** — multi-result return.
- **`io.flush` + getline / fwrite return value** — buffer
  control.
- **`print(io.read())` direct** — Builtin-TaggedValue source
  dispatch in Print/io.write arms.
- **Non-literal format strings** — runtime dispatch.
- **`io.open` / `io.close` / file handles** — Phase 3.
- **macOS / Windows libc** — cross-platform.
- **Resource policy** — getline OOM strict handler.

## Phase tag

`2.7x-stdlib-io` (write + read pair; extends ADR 0116 row).
