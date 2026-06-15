# 0197. Integer Literal Token — Additive Lexer Distinction (Phase A)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

[ADR 0196](0196-integer-float-subtype-design.md) §Sub-ADR decomposition §0197 designates this ADR for "Lexer/parser integer vs float literal distinction". Per §Migration §Phase A, this ADR is **additive only** — the lexer learns to distinguish integer-syntax literals (`42`, `0xFF`) from float-syntax literals (`42.0`, `1e5`) at the token level, but the parser/HIR/codegen pipeline silently demotes both to the existing f64 path so the 1431-test corpus stays green. Sub-ADRs 0198-0206 lift the demotion incrementally.

Current state (commit `854df26`):
- `src/lexer/mod.rs::scan_number` returns `(Span, f64)`. The function already detects the three lexical forms (hex `0x...`, decimal integer, decimal float with `.`/`eE` suffix) and routes hex through `u64::from_str_radix(...) as f64`.
- The syntactic distinction "did this literal have a `.` or `e/E`?" is computable inside `scan_number` with no extra lookahead.

Lua 5.4 §3.1 Lexical Conventions defines integer constants as `42` / `0xFF` and float constants as `42.0` / `1e5` / `0x1.8p3`. Hex floats (`0x1.8p3`) are still deferred per ADR 0023 — out of scope here.

## Scope (literal)

- ✅ New `TokenKind::Integer(i64)` variant alongside existing `TokenKind::Number(f64)`.
- ✅ `scan_number` returns one of two new outcomes: `Integer(i64)` for pure integer syntax (decimal digits OR `0x` hex digits, no `.`, no `eE`); `Number(f64)` for fractional / scientific notation.
- ✅ Parser accepts both token kinds. Both lower to the existing `ExprKind::Number(f64)` AST node via `i64 as f64` cast at the parser layer. The AST does **not** gain an integer variant in this ADR — that is ADR 0198.
- ✅ Lexer unit tests cover the new variant: `42` → `Integer(42)`, `0xFF` → `Integer(255)`, `42.0` → `Number(42.0)`, `1e5` → `Number(100000.0)`.
- ✅ All existing 1431 tests stay green via the silent conversion.
- ✅ Hex literals' overflow case: `u64::from_str_radix("FFFFFFFFFFFFFFFF", 16)` = `u64::MAX`. Cast to `i64`: bitcast preserves bits (i64 = -1). Documented; matches Lua 5.4 §3.1 integer overflow semantics for source literals.
- ❌ AST-level integer variant (`ExprKind::Integer(i64)`). Deferred to ADR 0198.
- ❌ HIR `ValueKind::Integer`. Deferred to ADR 0198.
- ❌ Hex float literals (`0x1.8p3`). Deferred (out of ADR 0196 scope too).
- ❌ Codegen i64 paths. Deferred to ADR 0199.
- ❌ Behavior change at any layer above the lexer. Phase A is additive at the token level only.

## Decision

### `src/lexer/mod.rs`

#### `TokenKind` extension

```rust
pub enum TokenKind {
    // ...existing variants...
    /// ADR 0197 — integer-syntax literal (decimal digits or
    /// `0x`-prefixed hex). Phase A additive: parser converts to
    /// f64 at use; HIR / codegen unchanged. ADR 0198 lifts the
    /// conversion by introducing `ExprKind::Integer` +
    /// `ValueKind::Integer`.
    Integer(i64),
    Number(f64),
}
```

#### `scan_number` signature change

Returns `(Span, NumericLit)` where `NumericLit` is:

```rust
pub(crate) enum NumericLit {
    Integer(i64),
    Float(f64),
}
```

Body logic:

1. Hex prefix path → `Integer(u64::from_str_radix(...).map(|u| u as i64))`. Bitcast `u64 → i64` so `0xFFFFFFFFFFFFFFFF` becomes `-1` per Lua spec.
2. Decimal scanner runs the integer part (digit run). If the loop exits without encountering `.` or `eE`, the lexeme is integer-typed; parse as `i64`. Overflow: `i64::from_str_radix(lexeme, 10)` returns `Err(IntOverflow)` → fall back to `f64::parse` for very large decimal integer literals (Lua spec falls back to float on integer overflow).
3. Otherwise (fractional or exponent present): float path, unchanged behaviour.

Token emission at `scan_number` callsite:

```rust
let (span, lit) = scan_number(src, chars);
let kind = match lit {
    NumericLit::Integer(i) => TokenKind::Integer(i),
    NumericLit::Float(f)   => TokenKind::Number(f),
};
tokens.push(Token::new(kind, span));
```

### `src/parser/mod.rs`

Adapt the existing `TokenKind::Number(f)` literal arm to also accept `TokenKind::Integer(i)`:

```rust
TokenKind::Number(value) => {
    let span = tok.span;
    self.bump();
    Ok(Expr::new(ExprKind::Number(*value), span))
}
TokenKind::Integer(value) => {
    let span = tok.span;
    self.bump();
    // ADR 0197 Phase A — silently demote to f64.
    // ADR 0198 introduces ExprKind::Integer; until then,
    // the AST is unchanged.
    Ok(Expr::new(ExprKind::Number(*value as f64), span))
}
```

### Tests

`src/lexer/mod.rs::tests` — add lexer-level unit tests:

1. `lex_integer_42` → `vec![TokenKind::Integer(42), TokenKind::Eof]`
2. `lex_integer_hex_ff` → `vec![TokenKind::Integer(255), TokenKind::Eof]`
3. `lex_float_with_decimal` → `vec![TokenKind::Number(42.0), TokenKind::Eof]`
4. `lex_float_scientific` → `vec![TokenKind::Number(1e5), TokenKind::Eof]`
5. `lex_integer_overflow_falls_back_to_float` — `99999999999999999999` lexeme → `TokenKind::Number(_)` (float fallback).
6. `lex_hex_max_u64_wraps_to_negative_i64` — `0xFFFFFFFFFFFFFFFF` → `TokenKind::Integer(-1)`.

Existing tests that assert `TokenKind::Number(42.0)` for `"42"` get updated to `TokenKind::Integer(42)`. The integer-syntax literal tests in the existing corpus are well-isolated (lexer mod tests, lines ~640-770 per current state); migration is mechanical.

No e2e test changes — the parser silent demotion preserves all 1431 tests' observable behaviour.

## Alternatives considered

- **Keep `scan_number` returning `(Span, f64)` and embed the integer/float bit elsewhere.** Rejected — losing the i64 representation at the lexer means the parser cannot recover precise integer values for large literals where f64 loses precision (e.g. `2^53 + 1`). Phase A additive should preserve full integer fidelity even if subsequent layers demote.
- **Skip the lexer change; do everything in the parser.** Rejected — the parser would have to re-scan the source-text of each numeric token to decide integer vs float. Lexer is the right layer; the scan is already there.
- **Treat hex literals as float (current behaviour) and only add integer variant for decimal.** Rejected — Lua 5.4 spec says `0xFF` is integer subtype. Distinguishing decimal but mis-classifying hex is worse than uniform.
- **Use `u64` instead of `i64` for the new variant.** Rejected — Lua 5.4 integers are signed 64-bit. Hex literals at the high end naturally wrap to negative; this is the spec-correct behaviour.

## Consequences

**Positive**
- Lexer-level integer/float distinction exists; subsequent ADRs build on it without re-lexing.
- Large integer literals (up to `i64::MAX`) preserve full precision through the token layer.
- Phase A additive contract: zero behaviour change above the lexer; 1431 tests stay green.

**Negative**
- Lexer test corpus gets updated for integer-typed assertions. Mechanical; ~20 token-list entries affected.
- `Token` enum grows by one variant. Trivial pattern-match exhaustiveness cost in parser arms.

**Locked in until superseded**
- `TokenKind::Integer(i64)` is the SoT for integer-syntax literals at the token layer. ADR 0198 adds the matching `ExprKind::Integer` + `ValueKind::Integer`; both consume `TokenKind::Integer`.
- `i64` over `u64` is the contract. Lua spec compliance.

## Documentation updates

- [x] §8 — adds 0197.
- [x] ADR 0196 §Sub-ADR decomposition §0197 — implementation done.

## Test count delta

```
Step 0: 1431 (after 670511d)
C1 (doc): 1431 → 1431
C2 (Red Day 0 lexer tests, 6 new + ~5 existing updates): 1431 → 1437
C3 (impl): 1431 → 1437 (all green)
```

## Critical files

- `docs/design/0197-integer-literal-token-additive.md` (this doc).
- `docs/design/README.md` index entry.
- `src/lexer/mod.rs`:
  - Add `TokenKind::Integer(i64)` variant.
  - Add `NumericLit { Integer(i64), Float(f64) }` enum.
  - Rework `scan_number` to return `(Span, NumericLit)`.
  - Update callsite to emit `Integer` or `Number` token kind.
  - Update mod tests (existing 5 + new 6).
- `src/parser/mod.rs`:
  - Parser literal arm accepts both `TokenKind::Integer` and `TokenKind::Number`; both produce `ExprKind::Number(f64)` (Phase A demotion).

## Risks

| Risk | Mitigation |
|---|---|
| Existing lexer tests break due to integer-syntax tokens now emitting `Integer` instead of `Number` | Mechanical migration in C2; the affected tests are co-located in `src/lexer/mod.rs::tests`. |
| Parser exhaustiveness panic on the new token variant outside the literal arm | Compile-time check; any `match TokenKind` without `Integer` arm fails to build. Caught immediately by `cargo build`. |
| `i64::from_str_radix` overflow on large decimal literals | Fall back to `f64::parse` per Lua spec; documented in §Decision and test 5. |
| Hex literals at `u64::MAX` wrap to `-1`. Source author might expect positive | Lua spec behaviour; documented in §Decision and test 6. |
| AST consumers (HIR, codegen) confused by integer-source f64 literals | Phase A bridge — they only see f64, exactly like today. No change. |

## Future work

- ADR 0198 — `ExprKind::Integer` + HIR `ValueKind::Integer` + `infer_kind` arms + arithmetic-result-kind rules (Phase B opt-in begins).
- ADRs 0199-0206 per ADR 0196 §Sub-ADR decomposition.

## References

- [ADR 0023](0023-phase2-2d-hex-float-literals.md) — original numeric lexer extension for hex and scientific notation.
- [ADR 0196](0196-integer-float-subtype-design.md) — design entry; this is its first implementation sub-ADR.
- [Lua 5.4 Reference Manual §3.1 Lexical Conventions](https://www.lua.org/manual/5.4/manual.html#3.1) — integer vs float literal definitions.
- [Lua 5.4 Reference Manual §3.4.3 Coercions and Conversions](https://www.lua.org/manual/5.4/manual.html#3.4.3) — integer overflow on source literal falls back to float.
