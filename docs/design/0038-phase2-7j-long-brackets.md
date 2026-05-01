# 0038. Phase 2.7j: Long-Bracket Strings & Level-N Block Comments

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** ShortArrow

## Context

Two related Lua surface forms remained unimplemented:

- **Long-bracket strings** `[==[ ... ]==]`. Multi-line string
  literals delimited by matching counts of `=` signs between `[`
  and `]`. Required for embedding Lua source containing `]]` (or
  `]=]`, etc.) without escape gymnastics, and for multi-line
  data blocks where short-string `\n` escape sequences would
  noise the source.
- **Level-N block comments** `--[=[ ... ]=]`. Phase 2.8c (ADR
  0034) explicitly deferred these to "the phase that lights up
  long-bracket strings", because both forms share one piece of
  grammar machinery: matching the count of `=` signs between
  the opening `[`s with the closing `]`s.

Doing them together avoids two near-duplicates of the level-N
matching logic.

## Decision

### One scanner, two callers

A single helper does the level-N matching:

```rust
fn try_match_long_open(bytes: &[u8], at: usize) -> Option<usize>;

fn scan_long_bracket_body<I>(
    chars: &mut Peekable<I>,
    bytes: &[u8],
    level: usize,
) -> Result<String, ()>
where
    I: Iterator<Item = (usize, char)>;
```

`try_match_long_open` is pure byte-index arithmetic — given the
source bytes and a position, it returns `Some(level)` if the
position starts a `[` + `=`*level + `[` sequence, else `None`.
Nothing is consumed.

`scan_long_bracket_body` walks `chars` until it sees a matching
`]` + `=`*level + `]` close, accumulating the body. The opening
brackets are consumed by the caller; the helper only sees the
body. Per Lua spec, an immediate leading `\n` after the opener
is dropped from the body. Returning `Err(())` on EOF lets the
caller pick the diagnostic variant
(`UnterminatedBracket` for strings, `UnterminatedComment` for
block comments).

### Two dispatch sites in `lex()`

```text
if ch == '-' && next == '-':
    consume the two `-`s
    if try_match_long_open at offset+2 is Some(level):
        consume `[==[`, scan_long_bracket_body (discard body),
        map Err to UnterminatedComment
    else:
        skip_line_comment

if ch == '[':
    if try_match_long_open at offset is Some(level):
        consume `[==[`, scan_long_bracket_body,
        map Err to UnterminatedBracket,
        push TokenKind::Str(body)
    else:
        fall through (bare `[` is still Unexpected until table
        indexing arrives)
```

`skip_block_comment` from Phase 2.8c (ADR 0034) is now a thin
wrapper that delegates to `scan_long_bracket_body` and discards
the body — no behaviour change for level-0 block comments, but
the level-N form becomes available for free.

### `LexError::UnterminatedBracket`

A new variant joins the existing comment / string / escape
diagnostics. It fires only when a long-bracket string opener
hits EOF before its closing `]==]`.

### No escape processing

Long-bracket strings are **raw** — `\n` inside `[[...]]` is
literal backslash + n, not LF. This matches Lua semantics and
is the whole point of the form (no escape gymnastics for code
samples and pasted text). Short-string escape processing in
`scan_string` is unchanged.

### CA invariants preserved

| Layer    | Change                                                  |
|----------|---------------------------------------------------------|
| Lexer    | Two pure helpers; new `UnterminatedBracket` variant; two-line dispatch tweak in `lex()`; `skip_block_comment` becomes a thin wrapper |
| Parser   | None                                                    |
| AST      | None                                                    |
| HIR      | None                                                    |
| Codegen  | None                                                    |

The body string flows through `TokenKind::Str(s)` exactly like
short-string literals, so the parser, AST, HIR, and codegen
treat long-bracket strings as ordinary strings — the only
difference is what the lexer accepted.

## TDD Process

1. **Tidy First** (commit 1): extracted `scan_long_bracket_body`
   and `try_match_long_open` from the existing block-comment
   scanner. The level-0 comment form was rewritten as
   `scan_long_bracket_body(chars, bytes, 0)` with the result
   discarded. Test count unchanged at 517 → behaviour preserved.

2. **Red**: 8 lexer unit tests + 12 integration tests added
   referencing the not-yet-existent `LexError::UnterminatedBracket`.
   The compiler refused to build (`E0599: no variant named
   UnterminatedBracket`), the canonical "no test passes because
   the production code can't see the variant" Red signal.

3. **Green**: added `UnterminatedBracket`, wired the `[`
   dispatch site, mapped the body scanner's `Err` to the right
   variant. The level-N comment form lit up automatically once
   the dispatcher's `try_match_long_open` call accepted any
   level (it was already calling the level-aware
   `scan_long_bracket_body`). Tests passed at 537 (517 + 8 + 12).

4. **Refactor**: cleaned up dead `close_end` arithmetic in the
   `[` dispatch — the body scanner's exit position can be read
   directly from `chars.peek()`, no separate offset bookkeeping.

## Alternatives Considered

- **Emit `LongString { level, body }` as a distinct token**.
  Useful only if downstream cares about the original surface
  form (e.g. a formatter). Rejected — `TokenKind::Str(String)`
  is sufficient for the compile path.
- **Process `\n` and `\\` inside long-bracket strings**. Would
  diverge from Lua semantics; the entire raison d'être of the
  form is to *avoid* escape processing. Rejected.
- **Open-coded second copy of the scanner for strings**. Would
  duplicate ~25 lines of lookahead logic. Rejected — Tidy First
  on the level-0 comment scanner was specifically about making
  this share possible.

## Consequences

- Lexer adds ~40 lines net (two helpers + dispatcher tweaks).
- One new `LexError` variant.
- 8 lexer unit tests + 12 integration tests covering: basic
  long string, leading-`\n` strip, internal `\n` preservation,
  level-1 / level-2 forms, no-escape semantics, concat with
  short string, `#` length, in-function use, level-1 block
  comment, mixed comment-around-print-with-long-string,
  unterminated diagnostic.
- Phase 2.8c's "Out of Scope: level-N brackets" item is
  retired.

## Out of Scope

- **Long-bracket strings as table keys** — pending tables.
- **Source-position metadata that distinguishes long-bracket
  from short-string tokens** — the lexer collapses both into
  `TokenKind::Str`. A formatter would need a token-preserving
  variant of `lex()`.
- **Shebang `#!`** at the start of a script (still deferred
  from Phase 2.8c).
