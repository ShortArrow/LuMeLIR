# 0022. Phase 2.2c: Floor Division and Bitwise Operators

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.2a shipped the standard arithmetic operators (`+ - * / % ^`)
but explicitly deferred `//` (floor division). Phase 2.4's note about
the standalone `~` token described it as reserved for "Phase 2.4
bitwise NOT" — that bucket never landed because each preceding phase
focused on control flow and functions. Phase 2.2c sweeps both up
together: the remaining Lua 5.3+ scalar operators are mechanical to
implement on top of the existing f64-typed expression world and
unblock natural Lua patterns (bit-packing, fixed-point arithmetic).

In scope:

- `//` floor division.
- Binary bitwise: `&`, `|`, `~` (XOR), `<<`, `>>`.
- Unary bitwise NOT `~`.

Out of scope:

- Hex / binary number literals (`0xff`, `0b1010`) — separate phase.
- String concatenation `..` and string length `#` — they share an
  AST tier with bitwise but need string-typed values; deferred.
- Lua's split between `integer` and `float` numeric subtypes — every
  number in our subset is `f64`. Bitwise ops convert via `fptosi` /
  `sitofp` at codegen time without trying to detect integer-valued
  floats.

## Decision

### 1. Lexer: six new tokens, one repurposed

`Tilde` (standalone `~`) was previously rejected as a lex error
("reserved for Phase 2.4"). The token now exists and surfaces as
both binary XOR and unary NOT, disambiguated by parser context.
`~=` keeps its own dedicated token.

| Token         | Lexeme |
|---------------|--------|
| `SlashSlash`  | `//`   |
| `Amp`         | `&`    |
| `Pipe`        | \|      |
| `Tilde`       | `~`    |
| `LtLt`        | `<<`   |
| `GtGt`        | `>>`   |

The two-character lex helper handles the four prefixes that have a
two-char form: `=`/`~`/`<`/`>`/`/`. Single `&` and `|` go through
the single-char branch.

### 2. AST: extend `BinOp` and `UnaryOp`

```rust
enum BinOp { ..., FloorDiv, BitAnd, BitOr, BitXor, Shl, Shr }
enum UnaryOp { ..., BitNot }
```

### 3. Parser: a new precedence tier between comparison and additive

Per Lua 5.4 §3.4.8:

```
or
and
<     >     <=    >=    ~=    ==
|
~     -- (XOR)
&
<<    >>
..    -- string concat (deferred)
+     -
*     /     //    %
unary not   #     -     ~
^
```

We introduce four constants — `PREC_BOR`, `PREC_BXOR`, `PREC_BAND`,
`PREC_SHIFT` — between `PREC_CMP` and `PREC_ADD`. `PREC_UNARY` and
`PREC_POW` shift up by four to keep the relative order. `//` joins
`*`/`/`/`%` at `PREC_MUL`. The unary `~` is recognised in
`parse_unary` alongside `-` and `not`.

### 4. HIR: type-check Number on both sides

Bitwise / shift / floor-div get folded into the existing arithmetic
arm of `lower_expr::BinOp` — both operands must be `Number`,
otherwise `TypeMismatch`. `binop_symbol` and `infer_kind` each gain
the new variants returning `Number`. `UnaryOp::BitNot` is added to
the unary arm with kind `Number`.

No new HIR variants — bitwise stays an ordinary `HirExprKind::BinOp`.

### 5. Codegen: f64 ↔ i64 round-trip via `fptosi` / `sitofp`

```rust
fn emit_f2i(...) -> Value  // arith.fptosi : f64 → i64
fn emit_i2f(...) -> Value  // arith.sitofp : i64 → f64
```

`emit_binop`'s bitwise / shift arm: `f2i lhs`, `f2i rhs`, run the
matching `arith.{andi, ori, xori, shli, shrsi}`, then `i2f`. `Shr`
uses `shrsi` (signed/arithmetic) — Lua 5.3 specifies arithmetic
shift for `>>` on integer values; our truncation already preserves
the sign bit.

`FloorDiv` reuses the existing libm `floor` declaration: emit
`arith.divf`, then `llvm.call @floor` on the quotient.

`UnaryOp::BitNot` is `f2i`, XOR with the i64 constant `-1`, `i2f`.

### 6. No verifier or runtime support changes

The bitwise lowering uses ops already enabled by
`--convert-arith-to-llvm` in the existing pipeline. No new external
declarations or globals.

## Alternatives Considered

- **Track integer vs float as a value-kind subtype** (true Lua 5.4
  semantics). Larger surface and not strictly needed for current
  tests. Rejected.
- **Run bitwise ops on the f64 bit pattern via `arith.bitcast`**
  (i.e. treat the IEEE 754 bits as the integer). Doesn't match Lua
  semantics — Lua converts the value, not the encoding. Rejected.
- **Deferring unary `~` to a later phase.** The lexer already had a
  reserved-error site; flipping it on at the same time as the
  binary form costs almost nothing extra. Adopted.

## Consequences

- Lexer: standalone `~` is now valid; the `lex_tilde_alone_returns_unexpected_error`
  test was rewritten to accept the new `Tilde` token.
- AST: `BinOp` grows six variants, `UnaryOp` grows one.
- HIR: arithmetic arm covers six new ops; unary arm covers one;
  `binop_symbol` / `infer_kind` updated.
- Codegen: two new helpers (`emit_f2i`, `emit_i2f`) and one new
  bitwise / shift arm in `emit_binop`. `emit_unary` now takes
  `context` and `types` because `BitNot` needs an i64 constant and
  the round-trip helpers.
- Parser: Pratt-precedence constants renumbered (`PREC_UNARY` /
  `PREC_POW` shifted up by four). No public API change.

## Out of Scope

- **Hex / binary literals** (`0xff`, `0b1010`).
- **String concatenation `..`** and string length `#` — share the
  bitwise tier in the precedence ladder but need string-typed values.
- **Integer subtype** (Lua 5.4's integer/float split) — every
  number stays `f64`; bitwise ops truncate via `fptosi`.
- **Overflow / NaN handling on `fptosi`** — out-of-range floats
  produce target-defined `i64`. Lua spec demands an "out of range"
  error for `math.tointeger` on such values; we currently truncate
  silently. A future ADR can add the runtime guard if needed.
