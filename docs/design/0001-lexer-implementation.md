# 0001. Lexer Implementation: Hand-Written

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

## Context

Phase 1 PoC must tokenize a Lua source (starting from `print(1 + 2)`) before the parser, HIR, and MLIR-emission stages can be exercised. We need to pick a lexer implementation strategy that:

- Keeps the Phase 0 "minimal dependencies" stance honest (see AGENTS.md §8)
- Scales to full Lua 5.4 lexical grammar — long brackets `[[ ... ]]`, `[=[ ... ]=]`, long comments `--[[ ... ]]`, hex/decimal/exponent numeric literals, `::label::`, and escape sequences in short strings
- Gives clear, source-position-accurate error reporting (MLIR diagnostics will consume these spans later)
- Stays idiomatic for the FP-leaning, pure-function core described in AGENTS.md §4.1

## Decision

**Write the lexer by hand** in `src/lexer/`. No external lexer crate in Phase 1.

Shape:

```rust
pub fn lex(src: &str) -> Result<Vec<Token>, LexError>
```

- Pure function: `&str` in, `Vec<Token>` out, no I/O, no global state.
- A small internal `Cursor<'a>` struct tracks byte offset and line/column for span construction. It is an implementation detail, not a public API — the functional shape stays intact at the module boundary.
- Token type carries a `Span { start: usize, end: usize }` for downstream diagnostics.

## Alternatives Considered

### `nom` (parser-combinator crate)
Rejected. Combinators are elegant for context-free token sets but get clumsy on Lua's nested long brackets where the closing delimiter depends on the opening (`[==[` must match `]==]`). Reimplementing this as a combinator obscures what is fundamentally a stateful scan. Also: adds a non-trivial transitive dependency graph for work we can do in ~300 lines.

### `logos` (derive-macro lexer)
Rejected. `logos` shines for regular languages, but Lua long brackets and long comments are **not regular** (the `=` count must be matched). We would end up dropping into a hand-written escape hatch for the non-regular parts, defeating the purpose of the derive. The proc-macro build-time cost is also non-trivial for a PoC.

### `chumsky`
Rejected. Excellent error recovery, but overkill for a lexer and aimed more at parser-level work. Revisit for the parser layer in a later ADR if recovery becomes a priority.

## Consequences

**Positive**
- Zero new dependencies in Phase 1 lexer work. Phase 0's "clap only" invariant survives one more step.
- Full control over span tracking and error messages from day one.
- Nested long brackets / comments are straightforward to handle with a counter, not awkward through a combinator.
- Easy to keep the public surface pure and testable (see AGENTS.md §4.3 TDD cycle).

**Negative**
- More lines of code than a derive macro would produce. Mitigated by strict TDD — each token class gets tests before implementation.
- We reinvent standard lexer machinery (keyword table, number parsing). Accepted cost given Lua's well-known grammar.

**Locked in until superseded**
- Future parser-layer decisions are independent of this ADR.
- If the lexer grows past ~800 lines or the number-literal code becomes error-prone, revisit `logos` for the *regular* subset and keep long-bracket handling hand-written (hybrid).
