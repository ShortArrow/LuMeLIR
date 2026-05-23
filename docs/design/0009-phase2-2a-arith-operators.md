# 0009. Phase 2.2a: Arithmetic Operators (`-`, `*`, `/`, `%`, `^`, unary `-`)

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Through Phase 2.1 the only arithmetic operator is binary `+`. Adding the
remaining Lua arithmetic operators is mechanical for the lexer/parser/HIR
but has two real decisions for codegen and one for parser shape:

1. **`/` semantics.** Lua 5.4 distinguishes `/` (always float) from `//`
   (floor division). All LuMeLIR numbers are `f64` already, so `/` is a
   straight `arith.divf`. `//` is deferred (Phase 2.2b or later).
2. **`^` and `%` need libm.** `arith` has `remf` (truncating) but Lua's
   `%` is *floor* modulo (`a % b == a - floor(a/b)*b`). And `^` has no
   MLIR builtin — pow lives in libm.
3. **Unary `-`.** Requires a new `ExprKind` variant; `-x` cannot be
   modelled as `0 - x` without losing the source span and risking
   surprising behaviour for `-0.0`.

This ADR locks the choices so 2.2a is purely additive — no rework of
existing ops or storage.

## Decision

### 1. Lexer

Add five `TokenKind`s: `Minus`, `Star`, `Slash`, `Percent`, `Caret`.
Each is a one-character punctuation token. `//` (floor div) and `**`
are not recognised; the lexer treats `**` as `Caret Caret` if it ever
appeared, but Lua source never produces it.

### 2. Parser: Pratt precedence + right assoc + unary

Extend `BinOp` with `Sub`, `Mul`, `Div`, `Mod`, `Pow`. Add
`UnaryOp::Neg` and `ExprKind::UnaryOp { op, operand }`.

Precedence ladder (Lua 5.4 §3.4.8 subset; lower number binds looser):

| Level | Operators        | Assoc |
| ----- | ---------------- | ----- |
| 10    | `+`, `-`         | left  |
| 11    | `*`, `/`, `%`    | left  |
| 12    | unary `-`        | —     |
| 13    | `^`              | right |

Right-assoc `^` means `2^3^2 == 2^(3^2) == 512`. Implement by passing
`prec` (not `prec + 1`) for the recursive call when the op is right-assoc.

Unary `-` is handled in `parse_primary` before the operand: if the
current token is `Minus`, consume it and recurse at level 12. This
makes `-x ^ 2` parse as `-(x^2)` (Lua's precedence: `^` binds tighter
than unary `-`).

### 3. HIR: pass-through

Mirror `BinOp` and add `HirExprKind::UnaryOp { op, operand }`. No new
errors. Lowering is identity-shaped.

### 4. Codegen

| Op   | Emit                                                       |
| ---- | ---------------------------------------------------------- |
| `-`  | `arith.subf`                                               |
| `*`  | `arith.mulf`                                               |
| `/`  | `arith.divf`                                               |
| `%`  | `a - floor(a/b)*b` via `arith.divf` + libm `floor` + `mulf` + `subf` |
| `^`  | `llvm.call @pow(a, b)` (declare extern `pow : (f64,f64) -> f64`) |
| u`-` | `arith.negf`                                               |

Add libm declarations alongside the existing `printf`:

```mlir
llvm.func @pow(f64, f64) -> f64
llvm.func @floor(f64) -> f64
```

Link with `-lm` in addition to `-lc`. (Currently `link.rs` only passes
`-lc`; this ADR adds `-lm` unconditionally.)

**Why `arith.remf` is wrong for `%`.** `arith.remf` is truncating
remainder (sign follows dividend); Lua's `%` is floor modulo (sign
follows divisor). `5 % -3` is `-1` in Lua, but `2` under truncating
remainder. Synthesizing via `floor` is two extra ops and matches Lua
exactly.

### 5. Linker change

`src/codegen/link.rs` currently invokes `cc -o <out> <obj> -lc`. Phase
2.2a appends `-lm`. No new flag plumbing — the math libs are unconditional.

## Alternatives Considered

- **Use only `arith` ops, no libm.** Rejected — there is no `arith.pow`,
  and `arith.remf` does not match Lua's `%` semantics.
- **Defer `%` and `^` to Phase 2.2b alongside comparisons.** Rejected —
  `%` and `^` don't need a truthiness model; they're orthogonal to the
  i1/bool decision that makes 2.2b interesting. Bundling them just delays
  shippable progress.
- **Model unary `-` as `BinOp(Sub, 0, x)`.** Rejected — span info would
  be wrong for diagnostics, and `-0.0 vs 0.0` semantics differ subtly.
- **Lazy `^` lowering through MLIR's `math.powf`.** Considered, but
  `math` dialect requires another conversion pass to LLVM. Direct libm
  `pow` keeps the pipeline single-step.

## Consequences

- `BinOp` grows by 5 variants; all existing matches need new arms (or
  reuse `arith` constructor parametrised by op).
- New `ExprKind`/`HirExprKind`/codegen path for `UnaryOp`.
- `link.rs` always passes `-lm` (new dependency). Toolchain check: WSL2
  Arch already has libm via glibc, and the existing GitHub Actions image
  ships it.
- Tests grow by ~5 lexer, ~6 parser (precedence/assoc), ~3 hir, ~6
  codegen (one per op), +1 e2e.

## Out of Scope (deferred)

- `//` floor division → Phase 2.2b
- Bitwise `&`, `|`, `~`, `<<`, `>>` → Phase 2.4+
- String concat `..` → Phase 2.5 (string runtime)
- Comparisons (`<`, `<=`, `==`, …) and Lua truthiness → Phase 2.2b
