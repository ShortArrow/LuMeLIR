# 0116. Phase 2.7x-stdlib-io-write: `io.write(...)` variadic stdout

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-22 (commit `eb93d77`; follow-up fix `71cbbae`)
- **Deciders:** ShortArrow

## Replan provenance

ADR 0115 (`arg` table) landed first per the codex-recommended
D bundle (`arg` + `io.write`, A → B sequence). ADR 0116
delivers the B half: `io.write(...)` — sibling of `print`
without the tab separator and without a trailing newline.

`io.write` is the **first `io.*` builtin** and the 4th consumer
of ADR 0103's `Builtin::from_namespace_method` generic
dispatcher (math + string + table + io = 4 namespaces).

## Codex critical fixes baked in

1. **`emit_print_value_raw` sibling, no new chokepoint** — the
   ADR 0064/0065/0074 TaggedValue-source dispatch pattern from
   Print is mirrored without re-extracting helpers.
2. **Bool / Nil HIR-time reject** — Lua §6.6 specifies only
   Number or String args. The standard ADR 0110 kind check
   skips via `TaggedValue` sentinel; an IoWrite-specific
   follow-up loop enforces the Number|String|TaggedValue
   restriction.
3. **Variadic** — `arity (0, usize::MAX)` matches Print
   precedent.
4. **Void return** — `ret_kinds = &[]`, expression-position
   placeholder f64 0.0 (Print precedent). Lua spec returns the
   file handle but MVP keeps void.
5. **`multi-result builtin framework` rejected** — `Builtin::
   ret_kinds` static-slice ABI stays single-shape; multi-result
   framework is speculative until 2nd consumer arrives.
6. **Index dispatch follow-up fix** (`71cbbae`) — initial
   commit's IoWrite arm gated TaggedValue source dispatch
   under `infer_kind(a) == TaggedValue`, but `arg[1]` is
   statically Number (HIR limitation). Print's ADR 0065 fix
   handles `HirExprKind::Index` regardless of static kind;
   the IoWrite arm now matches.

## Non-goals (top-of-ADR)

- **`io.read`** — separate ADR (0119); interactive harness +
  resource policy differ from `io.write`.
- **`io.open` / `io.close` / file handles** — Phase 3.
- **`io.flush` + `fwrite` return value** — small follow-up ADR.
- **`Builtin::ret_kinds` arity-dependent framework** — premature;
  `next` 1 case is enough.
- **Format-string variadic / `string.format`-like dispatch** —
  separate ADR for `string.format`.
- **`print(io.read())` direct** — Print/io.write Builtin-
  TaggedValue source dispatch (separate ADR 0118-like
  follow-up).
- **Embedded NUL stdout** — at the time of ADR 0116, stdout
  still truncated at NUL via `printf("%.*s")`; ADR 0117
  resolves this carry-over with the fwrite chokepoint swap.

## Goals

1. `Builtin::IoWrite` variant + `io_from_method("write")` arm
   + `"io"` namespace in `from_namespace_method`.
2. arity `(0, usize::MAX)`, ret_kinds `&[]` (void).
3. HIR-time reject Bool / Nil; Function / Table already
   rejected by `FunctionUsedAsValue` walks.
4. Codegen emit arm: per-arg `emit_print_value_raw` without
   tab / newline.
5. Test corpus: 1204 → 1216 (+12); follow-up fix +1 = 1217.

## Lua 5.4 §6.6 compliance

- `io.write(...)`: write each arg to stdout. No separator. No
  trailing newline.
- Accepts Number (coerced via `%g`) and String only.
- Returns the file handle; MVP returns void.
- Bool / Nil / Function / Table → "bad argument" error.

## 設計

### HIR

```rust
enum Builtin {
    IoWrite,
}

impl Builtin {
    pub fn io_from_method(method: &str) -> Option<Builtin> {
        match method {
            "write" => Some(Builtin::IoWrite),
            _ => None,
        }
    }

    pub fn from_namespace_method(ns: &str, method: &str) -> Option<Builtin> {
        match ns {
            "math" => Self::math_from_method(method),
            "string" => Self::string_from_method(method),
            "table" => Self::table_from_method(method),
            "io" => Self::io_from_method(method),
            _ => None,
        }
    }

    pub fn arity(self) -> (usize, usize) {
        match self {
            Builtin::IoWrite => (0, usize::MAX),
            // ...
        }
    }

    pub fn ret_kinds(self) -> &'static [ValueKind] {
        match self {
            Builtin::IoWrite => &[],  // void
            // ...
        }
    }

    pub fn expected_param_kind(self, _argc: usize, _pos: usize) -> Option<ValueKind> {
        match self {
            // Any-accepted sentinel (TaggedValue); IoWrite-
            // specific Bool/Nil reject runs in
            // `lower_namespace_builtin_call`.
            Builtin::IoWrite => Some(ValueKind::TaggedValue),
            // ...
        }
    }
}
```

`infer_kind` for IoWrite returns `ValueKind::Number` placeholder
(void; Print precedent for expression-position synthesis).

### HIR IoWrite-specific reject (`lower_namespace_builtin_call`)

```rust
if matches!(builtin, Builtin::IoWrite) {
    for (i, lowered) in lowered_args.iter().enumerate() {
        let actual = infer_kind(lowered, ...);
        match actual {
            ValueKind::Number | ValueKind::String | ValueKind::TaggedValue => {}
            _ => return Err(HirError::BuiltinArgKindMismatch {
                builtin: "io.write".into(),
                arg_index: i + 1,
                expected: "number or string".into(),
                actual: actual.name().to_owned(),
                offset: call_span.start,
            }),
        }
    }
}
```

### Codegen `Callee::Builtin(IoWrite)` arm

Mirror of Print arm (`emit.rs` Print code) without the tab
separator (line 7402-7404 removed) and without the trailing
newline (line 7490 removed).

Source dispatch (post-`71cbbae` fix):

- `HirExprKind::Local(idx)` + `infer_kind = TaggedValue` →
  `emit_print_tagged_local(slots[idx])`
- `HirExprKind::Index { target, key }` (regardless of static
  kind) → `emit_inline_index_into_tagged_tmp` + tagged print
  (ADR 0065 pattern; static kind for `arg[1]` is Number but
  the slot can hold any tag at runtime)
- `HirExprKind::Call { Callee::User { fid, holding_local }, ...
  }` returning TaggedValue → `emit_call_user_into_tagged_tmp` +
  tagged print
- otherwise → `emit_expr` + `emit_print_value_raw(v, kind)`

Returns f64 0.0 placeholder (Print/TableInsert precedent).

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `emit_print_value_raw` | `src/codegen/emit.rs:11235-11241` (post-0117) | per-arg print (no newline) |
| `emit_print_tagged_local` | `src/codegen/tagged.rs` | TaggedValue Local source dispatch |
| `emit_inline_index_into_tagged_tmp` | `src/codegen/emit.rs` | TaggedValue Index source dispatch (ADR 0065) |
| `emit_call_user_into_tagged_tmp` | `src/codegen/emit.rs` | TaggedValue User-call source dispatch (ADR 0074) |
| `Builtin::from_namespace_method` (ADR 0103) | `src/hir/ir.rs:526-533` | namespace dispatch |
| `BuiltinArgKindMismatch` (ADR 0110) | `src/hir/` | HIR error variant |
| Print arm precedent | `src/codegen/emit.rs` (Print) | variadic per-arg + newline pattern |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: extends existing namespace
  dispatcher, no new chokepoint.
- [x] **#2 TDD**: 12 e2e + 1 follow-up regression pin
  (`io_write_tagged_index_source_dispatches_on_runtime_tag`).
- [x] **#3 FP**: HIR-time validation is pure; codegen is
  effectful printf.
- [x] **#4 CA**: `src/cli/`, `src/pipeline.rs`, `src/parser/`,
  `src/lexer/`, `src/codegen/primitive.rs`, `src/codegen/tagged.rs`
  **zero-diff**.
- [x] **#5 Security**: Bool / Nil / Function / Table rejected
  at HIR; attack surface unchanged.
- [x] **#6 Documentation**: phase tag `2.7x-stdlib-io-write`
  (4th stdlib namespace lane; sibling of math/string/table).

## Test count delta

1204 → 1217 (+13 net: 12 ADR 0116 + 1 follow-up fix).

12 tests in `tests/phase2_stdlib_io.rs`:

| Test | Category |
|---|---|
| `io_write_basic_string` | happy |
| `io_write_multiple_strings_concatenate` | happy |
| `io_write_empty_arity_zero` | arity |
| `io_write_number_coerces_to_string` | coerce |
| `io_write_mixed_number_and_string` | mixed |
| `io_write_embedded_nul_loses_bytes_after_nul` | ABI carry-over pin (later inverted by ADR 0117) |
| `io_write_no_trailing_newline` | semantics |
| `io_write_then_print_adds_newline` | sibling diff |
| `io_write_rejects_bool` | HIR negative |
| `io_write_rejects_nil` | HIR negative |
| `io_write_rejects_table_literal` | HIR negative |
| `io_write_shadowed_respects_user_table` | codex critical |

Follow-up fix `71cbbae`:
| `io_write_tagged_index_source_dispatches_on_runtime_tag` | regression pin for ADR 0065 pattern |

## Critical files

- `src/hir/ir.rs` (~25 LOC delta: variant + dispatch + arity +
  name + ret_kinds + expected_param_kind)
- `src/hir/mod.rs` (~25 LOC delta: infer_kind + Bool/Nil reject)
- `src/codegen/emit.rs` (~110 LOC: IoWrite emit arm)
- `tests/phase2_stdlib_io.rs` (NEW, ~210 LOC: 12 + 1 e2e)

**Zero-diff (CA invariant)**:
`src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/`,
`src/codegen/primitive.rs`, `src/codegen/tagged.rs`.

## Risks

| Risk | Mitigation |
|---|---|
| Bool / Nil reach codegen | HIR-time reject in
`lower_namespace_builtin_call` |
| TaggedValue Index source extracted as f64 (static Number kind) | ADR 0065 pattern fix in follow-up commit `71cbbae` |
| Embedded NUL stdout truncation | Documented as ADR 0112 carry-over; resolved by ADR 0117 |
| `io.write()` return value expected by user | Documented MVP void return |

## Future work

- **ADR 0117** — `emit_print_string_obj` fwrite chokepoint swap
  (resolves the NUL truncation carry-over).
  **RESOLVED by ADR 0117 (2026-05-22)**.
- **ADR 0119** — `io.read` companion (line input).
  **RESOLVED by ADR 0119 (2026-05-23)**.
- **`io.flush`** — buffer control + fwrite return value check.
- **`io.open` / file handles** — Phase 3.
- **`print(io.read())` direct** — Builtin-TaggedValue source
  dispatch in Print/io.write arms.

## Phase tag

`2.7x-stdlib-io-write` (4th stdlib namespace lane).
