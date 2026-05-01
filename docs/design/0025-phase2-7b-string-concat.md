# 0025. Phase 2.7b: String Concatenation `..` and Runtime Equality

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7a shipped string literals, `print(string)`, and `#s`. The
two missing pieces for usable string handling are concatenation
(`a..b`) and runtime equality (`s1 == s2`). Concat necessarily
crosses into heap territory because the result depends on both
operand contents at runtime; equality is purely a comparison so it
stays cheap.

This phase lands both, plus the call-site param-kind inference
extension that lets `local function greet(name) return "hi "..name
end` work without ceremony.

## Decision

### 1. Lexer: `DotDot` token

The two-character `..` joins the lexer's `=`/`<`/`>`/`~`/`/`/`.`
group. A standalone `.` remains a lex error — table field access
isn't in the current grammar, so the `.` lookahead can lock in
unambiguously on `..` without future-proofing.

### 2. AST / parser: `BinOp::Concat` at PREC_CONCAT

`BinOp::Concat` joins the binary operator enum. The Pratt ladder
gains `PREC_CONCAT = 13`, slotted between `PREC_SHIFT` (12) and
`PREC_ADD` (now 14) per Lua 5.4 §3.4.8. The infix entry is
**right-associative** so `a..b..c` parses as `a..(b..c)`. Existing
`PREC_*` constants for higher tiers shift up by one.

### 3. HIR: type-check + heterogeneous-eq fold

`lower_expr`'s `BinOp::Concat` arm requires both operands to be
`ValueKind::String`; anything else is `TypeMismatch`. The result
kind is `String`. `infer_kind` follows suit.

`Eq` / `Ne` already drop to runtime when both operands share a
non-Nil kind (Phase 2.3a). String slips into that path naturally
— no fold change is needed.

`ast_arg_kind` (the call-site param-kind pre-scanner from Phase
2.5e) now recognises `ExprKind::Str`, so a function called with a
string literal at its first call site has its param refined to
`ValueKind::String`. The existing call-site compatibility table
gains a `(String, String) → true` entry; everything else falls
through to `TypeMismatch`.

### 4. Codegen: `malloc` + `memcpy` + `strcmp` runtime

A new `emit_string_runtime_decls` declares libc `malloc(i64) → ptr`,
`memcpy(ptr, ptr, i64) → ptr`, and `strcmp(ptr, ptr) → i32` at
module top. Combined with the existing `strlen` declaration from
Phase 2.7a, that's the entire runtime surface.

`emit_concat`:

```text
la = strlen(lhs)
lb = strlen(rhs)
total = la + lb + 1
buf = malloc(total)
memcpy(buf, lhs, la)
memcpy(buf + la, rhs, lb + 1)   ; +1 copies rhs's NUL terminator
return buf
```

The `buf + la` offset uses `llvm.getelementptr` with an `i8`
element type. The result is a fresh ptr to a NUL-terminated string;
the buffer **leaks** intentionally — there is no GC yet, and string
churn in the test programs is bounded.

`emit_string_eq` calls `strcmp` and compares the i32 result against
zero with `arith.cmpi` (Eq for `==`, Ne for `~=`). The path mirrors
the Number case but uses `cmpi` rather than `cmpf`.

### 5. Bypass paths in `emit_expr`

`HirExprKind::BinOp` intercepts `Concat` and String-kind `Eq`/`Ne`
before reaching `emit_binop`, since the existing `emit_binop`
machinery is f64-typed and can't accept `ptr` operands. The Concat
arm in `emit_binop` itself is `unreachable!()` to make the contract
explicit.

Three small helpers — `emit_libc_call_i64`, `emit_libc_call_i32`,
`emit_libc_call_ptr` — wrap the boilerplate `llvm.call` operation
builder around result-typed extern decls. They reduce per-callsite
clutter and are local to this phase's runtime needs.

## Alternatives Considered

- **Static fold for `"a".."b"`** at HIR-time. Avoids a runtime
  malloc when both operands are literals. Defer — adds AST →
  HIR transform complexity for a small wins; LLVM may fold it
  itself once it sees the `strlen`/`memcpy` pattern with constant
  inputs.
- **Snprintf-style format-string concat** (`asprintf("%s%s",
  a, b)`). Pulls in `asprintf` (POSIX, not C99) and adds a free
  step. The current `malloc` + `memcpy` is portable and the same
  cost.
- **`strcat` instead of two `memcpy` calls**. `strcat(buf, a)` then
  `strcat(buf, b)` works but each call re-walks the destination's
  current length — quadratic on long concats. `memcpy` with cached
  lengths is linear.
- **Reference-counted strings or arena allocator**. Both are valid
  GC strategies, but Phase 2.6's table + GC design is the natural
  place to add either. Until then the leaked-malloc model is the
  simplest correct semantic.

## Consequences

- Lexer / parser: `DotDot` token, `BinOp::Concat`, `PREC_CONCAT`
  shifts every higher-tier `PREC_*` constant up by one.
- HIR: type-check arm for Concat, `ast_arg_kind` recognises
  `ExprKind::Str`, call-site compatibility table accepts
  `(String, String)`.
- Codegen: `emit_string_runtime_decls`, `emit_concat`,
  `emit_string_eq`, three `emit_libc_call_*` helpers, intercept
  paths in `emit_expr`'s BinOp arm.
- Twelve integration tests in `phase2_7b_string_concat.rs` cover
  literal-only concat, three-operand right-associativity, locals,
  `==` / `~=`, `#(a..b)`, concat result driving an `if`, the
  Number-vs-String reject, and concat inside a user function.

## Out of Scope

- **Auto-coerce numbers to strings** in concat (`"x"..1` evaluates
  to `"x1"` in stock Lua). Needs `tostring` runtime; defer to the
  string library phase.
- **Lexicographic comparison** (`<`, `<=`, `>`, `>=` on strings) —
  `strcmp` would handle it but adds a per-kind switch in the
  comparison codegen path. Defer.
- **Free / GC / refcount** on concat results. Currently every
  concat leaks; acceptable for a static AOT compiler without GC.
- **The `string` standard library**, method syntax (`s:upper()`),
  `tostring`, format strings.
