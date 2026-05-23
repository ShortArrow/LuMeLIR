# 0034. Phase 2.8c: Block Comments `--[[ ... ]]`

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.8a shipped single-line `-- ...` comments. The next-most
common Lua comment form is the block comment `--[[ ... ]]`,
which spans multiple lines. Without it, multi-line file headers
and block-disabled code (a common debugging pattern) need
per-line `--` prefixes.

This phase adds the level-0 block-comment form. The level-N
form `--[=[ ... ]=]` (with matched `=` sign counts) is the same
machinery as long-bracket strings (`[=[...]=]`) and is deferred
to that phase.

## Decision

### Lexer-only change, two pure helpers

Following the FP discipline from Phase 2.7h, comment scanning is
expressed as two pure helpers — one per shape — both depending
only on their arguments:

```rust
fn skip_line_comment<I: Iterator<Item = (usize, char)>>(
    chars: &mut Peekable<I>,
);

fn skip_block_comment<I: Iterator<Item = (usize, char)>>(
    chars: &mut Peekable<I>,
    open_offset: usize,
) -> Result<(), LexError>;
```

The dispatcher in `lex()` decides which one to call:

```text
if ch == '-' && next == '-':
    consume the two `-`s
    if next two are `[[`:
        consume them, skip_block_comment(chars, open_offset)
    else:
        skip_line_comment(chars)
    continue
```

`open_offset` (the byte offset of the leading `-`) is threaded
into the block-comment scanner so the
`LexError::UnterminatedComment { offset }` diagnostic points back
at the comment's start, not at EOF.

### `LexError::UnterminatedComment`

A new variant joins the existing `Unexpected`,
`UnterminatedString`, and `InvalidEscape`. It fires only when a
block comment hits EOF before its closing `]]`.

### What the lexer doesn't do (yet)

- **Level-N brackets** (`--[=[ ... ]=]`). Lua's full grammar
  matches the count of `=` signs between `[` and `]` to allow
  nesting. We support only the level-0 form (`--[[ ... ]]`); a
  `--[=[` is currently a lex error (the bare `--[` skip path
  fires, then `=` is `Unexpected`). When long-bracket strings
  arrive in a future phase the same matching machinery will
  unlock both forms in one go.

- **Nested `--[[ ... ]]`**. Lua doesn't allow nesting in the
  level-0 form either — `--[[ a --[[ b ]] c ]]` closes at the
  first `]]`, leaving `c ]]` as a stray token sequence. Our
  scanner matches that semantic.

- **`#!` shebang on the first line**. A separate concern; not
  yet handled.

### CA invariants preserved

| Layer    | Change                                           |
|----------|--------------------------------------------------|
| Lexer    | New variant; new `skip_block_comment`; one-line dispatch in `lex()` |
| Parser   | None                                             |
| AST      | None                                             |
| HIR      | None                                             |
| Codegen  | None                                             |

Comments remain indistinguishable from whitespace once `lex()`
returns — no tokens, no AST nodes, no HIR or codegen surface.

## TDD Process

The implementation followed Tidy First → Red → Green:

1. **Tidy First** (commit 1): the existing inline `-- ...` skip
   was lifted into `skip_line_comment`. Test count unchanged at
   472 → confirms behaviour-preserving.

2. **Red** (committed alongside Green for atomicity): five lexer
   unit tests + six integration tests added referencing the
   not-yet-existent `LexError::UnterminatedComment`. The compiler
   refused to build (`E0599: no variant named UnterminatedComment`),
   the canonical "no test passes because the production code
   can't see the variant" Red signal.

3. **Green** (commit 2): `LexError::UnterminatedComment` added,
   `skip_block_comment` helper added, dispatcher in `lex()`
   widened to choose between the two helpers. Tests passed at
   483.

## Alternatives Considered

- **Emit a `BlockComment(String)` token** that the parser
  ignores. Useful for future tooling (formatters, doc
  generators) but superfluous for the compile path. Rejected
  for now — when a tooling consumer arrives, a token-preserving
  variant of `lex()` can be added behind a feature flag.

- **Implement level-N brackets in the same phase**. Tied to
  long-bracket string scanning; deferred to that phase so both
  forms share one grammar-level decision.

- **Defer block comments entirely until tables / closures**.
  Block comments unblock multi-line file headers immediately
  and cost ~30 lines of lexer code. Rejected — no benefit to
  delaying.

## Consequences

- Lexer adds ~30 lines (two helpers + a dispatcher tweak).
- One new `LexError` variant.
- Five lexer unit tests + six integration tests covering inline,
  multi-line, comment-only file, mixed line/block coexistence,
  unterminated diagnostic, and the `--[ x` (single-bracket
  line-comment) edge case.

## Out of Scope

- **Level-N brackets `--[=[ ... ]=]`** — pending the long-
  bracket-string phase.
- **Token-preserving `lex()` mode** for formatters / doc tools.
- **Shebang `#!`** at the start of a script.
- **Nested block comments** — not in Lua's grammar.
