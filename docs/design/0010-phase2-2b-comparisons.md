# 0010. Phase 2.2b: Comparison Operators and Boolean Literals

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Phase 2.2a (ADR 0009) completed the arithmetic operator set. The next
prerequisite for `if`/`while` (Phase 2.3) is a way to **produce** and
**observe** boolean values. A condition expression that cannot be
evaluated and printed is impossible to test end-to-end without
control-flow constructs.

Phase 2.2b adds the smallest surface that makes booleans observable:

1. The six comparison operators (`<`, `<=`, `==`, `~=`, `>`, `>=`).
2. The `true` / `false` literals.
3. A `print` path that prints a boolean as the literal string `true` or
   `false` (matching Lua 5.4's `print` of a boolean).

Everything else about Lua's boolean and value model — `nil`, the
"falsy = `false`/`nil`, everything-else = truthy" rule, heterogeneous
`==`, short-circuit `and`/`or`/`not`, `local b = true` — is **deferred
to Phase 2.3+**. Those features only become observable once `if` exists.

## Decision

### 1. Lexer: 2-character lookahead for compound operators

Extend `TokenKind` with `Lt`, `Gt`, `LtEq`, `GtEq`, `EqEq`, `TildeEq`.
The single-character `match` block in `lex` is preceded by a 2-character
peek for `=`/`<`/`>`/`~`:

| First | Second | Token       |
| ----- | ------ | ----------- |
| `=`   | `=`    | `EqEq`      |
| `=`   | other  | `Equals`    |
| `<`   | `=`    | `LtEq`      |
| `<`   | other  | `Lt`        |
| `>`   | `=`    | `GtEq`      |
| `>`   | other  | `Gt`        |
| `~`   | `=`    | `TildeEq`   |
| `~`   | other  | `LexError::Unexpected { ch: '~' }` |

Standalone `~` is rejected. Lua 5.4 uses `~` for unary bitwise NOT
(Phase 2.4+); reserving it as an error now keeps lexer surface explicit.

### 2. Lexer: `true` / `false` keywords

Extend `Keyword` with `True`, `False`. Same post-processing path as
`local`/`do`/`end` — identifier scan, then `Keyword::from_lexeme`
classifies.

### 3. AST: `ExprKind::Bool` and 6 new `BinOp`s

```rust
pub enum ExprKind { ..., Bool(bool) }
pub enum BinOp    { ..., Lt, Le, Gt, Ge, Eq, Ne }
```

### 4. Parser: precedence and dispatch

A new precedence level `PREC_CMP = 8`, between `PREC_ADD = 10` and
the implicit minimum of `0`, all left-associative. (Lua 5.4 §3.4.8
places relational ops *below* arithmetic, which matches.)

`parse_primary` gains a `Keyword(True | False)` arm producing
`ExprKind::Bool(true | false)`.

### 5. HIR: minimal type discrimination

The HIR stays untyped at the value level, but a private helper
`infer_kind(&HirExpr) -> ValueKind { Number, Bool }` is introduced.
It is a syntactic walk:

| HirExprKind        | ValueKind |
| ------------------ | --------- |
| `Number(_)`        | `Number`  |
| `Bool(_)`          | `Bool`    |
| `Local(_)`         | `Number`  *(slots are f64 in 2.2b)* |
| `BinOp { op, .. }` | `Number` for arithmetic, `Bool` for comparisons |
| `UnaryOp::Neg`     | `Number`  |
| `Call { .. }`      | `Number` *(only `print` exists, treated as expression-like for now)* |

`HirError::TypeMismatch { op, lhs_kind, rhs_kind, offset }` is added.
Lowering rules:

- `<`, `<=`, `>`, `>=`: both sides must be `Number`. Bool ordering is
  a type error.
- `==`, `~=`: both sides must have the *same* `ValueKind`. Heterogeneous
  comparison is rejected. (Lua's official semantics — `1 == true`
  yields `false` — requires runtime type tags; defer.)

### 6. Codegen: `arith.cmpf` and i1 constants

`Types` gains `i1: Type<'c>`. `HirExprKind::Bool(b)` lowers to
`arith.constant <0|1> : i1`. `emit_binop` is split:

- `emit_arith_binop` (`Add`/`Sub`/`Mul`/`Div`/`Mod`/`Pow`) — unchanged
  shape, `f64 → f64 → f64`.
- `emit_cmp_binop` (`Lt`/`Le`/`Gt`/`Ge`/`Eq`/`Ne`) — `arith.cmpf`
  with **ordered** predicates `olt`, `ole`, `ogt`, `oge`, `oeq`, `one`.
  Result type is `i1`.

Ordered predicates make `NaN <op> x` always `false`, including
`NaN == NaN`. This matches IEEE 754 and is what Lua 5.4 specifies.

### 7. Codegen: `print(bool)` via `%s` + `llvm.select`

Three new module-level globals:

```mlir
llvm.mlir.global internal constant @fmt_str("%s\n\0")
llvm.mlir.global internal constant @s_true("true\0")
llvm.mlir.global internal constant @s_false("false\0")
```

`Builtin::Print` dispatches on `infer_kind(&args[0])`:

- **Number**: existing `printf("%g\n", v)` path.
- **Bool**: `%true_ptr = addressof @s_true`, `%false_ptr = addressof @s_false`,
  `%selected = llvm.select %v, %true_ptr, %false_ptr`,
  `%fmt = addressof @fmt_str`, `printf(%fmt, %selected)`.

`llvm.select` over `!llvm.ptr` avoids creating new basic blocks — keeps
`emit_main` single-block as today.

### 8. Storage unchanged

Stack slots remain `f64`. `local b = true` still rejects in HIR (the
RHS is `Bool` but HIR currently has no bool slot type). This is
**deliberate** — adding bool slots requires per-slot type tracking,
which is the natural carrier for 2.3's `if` work.

## Alternatives Considered

- **Full type inference pass.** Overkill for 2.2b. Re-evaluate after
  2.3 when more value kinds (nil, string) appear.
- **Encode bool as f64 0.0/1.0.** Print would yield `"0"`/`"1"`,
  diverging from Lua's `"true"`/`"false"`. Rejected.
- **Heterogeneous `==` always false.** Requires a dynamic type tag at
  the value level. Defer to 2.3+.
- **Reserve `~` as a placeholder token (NotYetImplemented).** Makes
  lexer surface ambiguous. Reject early; reintroduce in 2.4 with
  bitwise ops.
- **Add `local b = true` now.** Forces per-slot type tracking, which
  is the right addition for 2.3's `if` (where `nil`/`bool` slots make
  sense). Defer.

## Consequences

- `TokenKind` +6 (`Lt`, `Gt`, `LtEq`, `GtEq`, `EqEq`, `TildeEq`).
- `Keyword` +2 (`True`, `False`).
- `ExprKind` +1 (`Bool`).
- `BinOp` +6 (`Lt`/`Le`/`Gt`/`Ge`/`Eq`/`Ne`).
- `HirExprKind` +1, `HirError` +1 (`TypeMismatch`).
- `Types` +1 field (`i1`), 3 new globals, new `emit_print_value` split.
- `link.rs` unchanged (no new libm dependency).
- Test count grows by ~14 unit + 7 e2e.

## Out of Scope (deferred)

- `local b = true` (per-slot type tracking) → Phase 2.3
- `nil` literal, official Lua truthiness, `and`/`or`/`not` → Phase 2.3
- Heterogeneous `==` (Lua's `1 == true == false`) → Phase 2.3+
- `//` floor division → Phase 2.4+
- Bitwise `~`, `&`, `|`, `<<`, `>>` → Phase 2.4+
- String concat `..`, string runtime → Phase 2.5+
