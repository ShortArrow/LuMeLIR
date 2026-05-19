# 0024. Phase 2.7a: String Literals, `print(string)`, and `#` Length

- **Status:** Accepted (ABI surface superseded by ADR 0112)
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

> **Superseded ABI:** ADR 0112 (2026-05-19) replaced the
> `!llvm.ptr`-to-NUL-terminated-C-string representation defined
> here with a boxed string object (`{i64 len, i8 data[len+1]}`)
> to restore Lua spec-compliant byte-string semantics
> (embedded NUL correctness). The HIR `ValueKind::String`
> contract and the `#s` / `print(string)` user-visible surface
> are unchanged; only the codegen representation and consumer
> chokepoints (`emit_string_obj_len` / `_data` / `_eq` /
> `_compare` / `_hash`) were swapped.

## Context

Through Phase 2.6 the compiler had no way to express a string. Every
`print(...)` example had to use a number literal, even though Lua
programs are saturated with strings (`print("hello")`,
`if name == "init" then ...`, etc.).

This phase introduces the smallest viable string layer:

- Source-level string literals `"..."` and `'...'` with the basic
  escape set.
- A new value-kind `String` carried through the type system.
- `print(string)` via the existing `printf` path.
- The unary length operator `#s` (and `#"..."`).

Out of scope:
- Concatenation `..` and `==` between strings — Phase 2.7b.
- Numeric escape forms (`\65`, `\xff`, `\u{...}`) and the long-
  bracket `[[...]]` literal.
- The `string.*` standard library, methods (`s:upper()`), and
  numeric-to-string auto-coercion.

## Decision

### 1. Lexer: `Str` token + `Hash` punctuation

`TokenKind::Str(String)` carries the post-escape payload. A new
`scan_string` helper handles both `"..."` and `'...'`, reads the
basic escape set (`\n \t \r \\ \" \' \0`), and returns either:

- `LexError::UnterminatedString { offset }` — closing quote missing
  or eclipsed by a literal newline / EOF.
- `LexError::InvalidEscape { seq, offset }` — backslash followed by
  any byte outside the recognised set.

`#` lexes as a dedicated `TokenKind::Hash` token.

### 2. AST: `ExprKind::Str` and `UnaryOp::Len`

`ExprKind::Str(String)` slots into `parse_primary` next to
`Number`, `Bool`, `Nil`, `Ident`. `UnaryOp::Len` joins `Neg`,
`Not`, `BitNot`. The Pratt rule for unary `#` reuses the existing
`PREC_UNARY` entry — no precedence-table change.

### 3. HIR: `ValueKind::String`

`ValueKind` gains a single new variant (no payload — length isn't
tracked statically). `infer_kind` for `Str` returns `String`;
`UnaryOp::Len` returns `Number`. `lower_expr` type-checks `#x`'s
operand: anything other than `String` is `TypeMismatch`.

`String` propagates through:
- `local s = "..."` — slot kind `String`, no `func_id`.
- Function parameters: pass-through, kind `String`.
- Function returns: `ret_kinds` may now contain `String`.
- Cross-return arity / kind check (Phase 2.5d) covers it without
  new code.

### 4. Codegen: `!llvm.ptr` slots + static C-string globals

Each unique literal payload is hoisted into an `llvm.mlir.global`
named `lstr_<i>` at module top, NUL-terminated so it is directly
usable by libc. Deduplication is via `BTreeSet<String>` so the
emitted IR is deterministic regardless of source order.

`Types` gains a `string_pool: HashMap<String, String>` field
(payload → global name) populated before any `emit_*` runs. Every
`HirExprKind::Str(s)` site looks the payload up and emits a single
`llvm.mlir.addressof @lstr_<i>` returning a `ptr`.

`emit_alloca_slot_for_kind(String)` allocates a `ptr`-element slot.
String-kind locals store/load the pointer directly; no
`unrealized_conversion_cast` is needed because the slot type and the
SSA value type already match (unlike Function-kind, ADR 0019).

`param_mlir_type(String)`, `ret_mlir_types`, and `kind_to_mlir_type`
all map `String → ptr`. Function signatures and the trailing
`func.return` therefore handle String returns by loading the slot
without a bridge cast.

### 5. `print(s)` and `#s`

`emit_print_value` extends to dispatch on `String`: load the
pre-existing `fmt_str` global (`"%s\n"`) and call `printf(fmt, s)`.

`emit_unary(Len)` calls libc `strlen` — declared once at module top
via the new `emit_strlen_decl` — and converts the resulting `i64`
back to `f64` with `arith.sitofp` so the value flows into our
Number-typed expression world. Static folding for literals is
intentionally not implemented; `strlen("hello")` runs at program
start, the cost is negligible, and the path is the same as for
locals.

### 6. Truthiness and HIR fold rules

In Lua every string is truthy (including `""`). `emit_truthiness`
returns the constant `i1 1` for `String`, mirroring the Number arm.

The heterogeneous `==` HIR fold (ADR 0011) folds `string == nil`
and `string == number` to a constant Bool(false). String == String
runtime equality is deferred to Phase 2.7b along with concat.

## Alternatives Considered

- **Track length statically** in a `(ptr, len)` struct slot. Avoids
  `strlen` calls. Rejected for now — `#` is rare in our benchmarks
  and `strlen` is plenty fast.
- **Heap-allocate each string copy**. Required for runtime concat.
  Deferred to Phase 2.7b; literal-only programs need no allocator.
- **One global per occurrence**, no dedup. Simpler emit but bloats
  the binary linearly with source size. The `BTreeSet` dedup costs
  one HashMap lookup per use site — trivially worth it.
- **Treat `print(s)` as a call site of a stdlib `tostring`**. Lua's
  actual semantics, but needs a stdlib table that we don't have.
  Direct `printf` dispatch is the same end-to-end behaviour for the
  literal subset we accept.

## Consequences

- Lexer: `TokenKind::Str(String)`, `TokenKind::Hash`, two new
  `LexError` variants.
- AST/HIR: `ExprKind::Str`, `UnaryOp::Len`, `HirExprKind::Str`,
  `ValueKind::String`.
- Codegen: `Types.string_pool`, three new helpers
  (`collect_string_pool`, `emit_user_string_globals`,
  `emit_strlen_decl`), plus String arms in `emit_alloca_slot_for_kind`,
  `emit_expr` (literal + Local), `emit_print_value`, `emit_unary`,
  `emit_truthiness`, `kind_to_mlir_type`, `param_mlir_type`,
  `ret_mlir_types`, and the `emit_function` trailing return.
- Twelve integration tests in `phase2_7a_strings.rs` covering the
  three observable behaviours (print, length, locals) plus error
  paths (invalid escape, unterminated, `#` on non-string).

## Out of Scope

- Concat `..` and string-equality runtime — Phase 2.7b.
- Numeric / unicode escape forms.
- Long-bracket strings `[[...]]`, multi-line literals.
- The `string.*` library, method syntax, format strings.
- Auto-coercion between numbers and strings.
