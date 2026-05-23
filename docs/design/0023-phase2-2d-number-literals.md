# 0023. Phase 2.2d: Hex Integer, Decimal Float, and Scientific Number Literals

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Through Phase 2.2c the lexer accepted only ASCII-digit-run integer
literals — every previous numeric example in the test suite was
`42` / `1` / `0`. Three Lua 5.4 numeric forms were missing:

- Hex integers: `0xff`, `0X1A`. Now naturally useful after Phase 2.2c
  introduced bitwise operators (`0xff & 0x0f`).
- Decimal floats: `3.14`, `0.5`.
- Scientific notation: `1e3`, `2.5e-1`, `2e+2`.

All three lex into our existing `f64` representation; no AST, HIR,
or codegen change is needed.

## Decision

### 1. `scan_number` becomes a small recogniser instead of a digit run

The previous implementation was a single `is_ascii_digit` loop. The
new version dispatches on the first byte:

- `0` followed by `x`/`X` and ≥1 hex digit → hex branch:
  `u64::from_str_radix(<digits>, 16) as f64`.
- Otherwise the decimal branch: integer digits, optional `.\d+`
  fractional part, optional `[eE][+-]?\d+` exponent. The whole
  lexeme is fed to Rust's `f64::parse`, which handles the IEEE 754
  rounding.

Lookahead is byte-based (`src.as_bytes()`) rather than iterator
clone, so we don't require `Clone` on the `chars` iterator type
parameter. The mutable `Peekable` advances once per accepted byte
to keep the caller's iterator in sync.

### 2. Conservative fractional / exponent recognition

A `.` is consumed only when **immediately** followed by a digit.
That avoids future ambiguity with table field access (`t.x`) — a
syntax we don't have yet but which is on the Phase 2.6 roadmap.

`e` / `E` is consumed only when followed by an optional sign and
≥1 digit, so a stray `e` (e.g. inside an identifier the upstream
caller already routed to `scan_ident`) doesn't get pulled in.

### 3. No subtype split

Lua 5.4 distinguishes `integer` and `float` subtypes; bitwise ops
require integer. Our subset stores every number as `f64`. `0xff`
parses as an integer (avoiding f64 rounding of large hex values up
to `2^53`) but the Token carries an `f64` payload immediately. This
matches the existing Phase 2.2c rule that bitwise operands round-
trip through `arith.fptosi` / `arith.sitofp`.

## Alternatives Considered

- **Full Lua 5.4 numeric semantics** with an integer/float subtype.
  Larger AST/HIR change, not blocking any test in the current
  trajectory. Rejected for now.
- **Hex floats `0xff.5p4`**. Lua-supported; a tractable extension
  of the same recogniser. Deferred — no current test demands it.
- **Binary literals `0b1010`**. Not in Lua's standard numeric
  grammar; deferred indefinitely.
- **Underscore digit separators `1_000`**. Not in Lua. Rejected.

## Consequences

- `scan_number` widens from "digit run" to a tiny FSM-style
  recogniser. Function signature is unchanged; only the body grows.
- Lexer test count grows by six (hex lower/upper, decimal float,
  two scientific forms, trailing-punctuation regression).
- E2E test file `tests/phase2_2d_number_literals.rs` adds ten tests
  covering hex/float/scientific in isolation, in arithmetic, and in
  bitwise expressions.

## Out of Scope

- Hex floats (`0xff.5p4`).
- Binary literals (`0b...`).
- Lua 5.4 integer subtype.
- Numeric overflow handling beyond what Rust's `f64::parse`
  produces (which infinitises out-of-range exponents).
