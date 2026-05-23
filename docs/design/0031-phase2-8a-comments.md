# 0031. Phase 2.8a: Lua Single-Line Comments

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Through Phase 2.7g the lexer rejected `--` (or treated the first
`-` as the unary/binary minus and then errored on the second).
Programs couldn't carry inline documentation, comment out a line
during debugging, or annotate test fixtures. Comments are the
single most basic ergonomic feature in any source language; this
phase lights up the line-comment form.

The long-bracket multi-line form (`--[[ ... ]]`) shares its
machinery with long-bracket strings and is deferred to a later
phase. Doc-comment conventions (`---`) are not currently part of
any tooling, so they piggy-back on the standard line-comment rule.

## Decision

### Lexer-only change

Right after the whitespace skip, the lexer checks for a `--`
prefix using the same `bytes.get(offset + 1)` byte-index lookahead
as the number scanner. When the prefix matches:

```rust
if ch == '-' && bytes.get(offset + 1) == Some(&b'-') {
    chars.next(); // first '-'
    chars.next(); // second '-'
    while let Some(&(_, c)) = chars.peek() {
        if c == '\n' { break; }
        chars.next();
    }
    continue;
}
```

The `\n` itself is left in the stream so the outer whitespace skip
on the next iteration consumes it normally. EOF terminates the
comment cleanly without a separate guard.

A bare `-` keeps its arithmetic-minus meaning — the prefix check
fires only when the second byte is also `-`.

### No changes above the lexer

`Token`, `TokenKind`, the AST, HIR, and codegen are all
untouched: from a parser/HIR perspective comments are
indistinguishable from whitespace.

## Alternatives Considered

- **Emit a `Comment` token** that the parser then ignores. Useful
  for source-preserving tooling (formatters, doc generators) but
  superfluous for the compile path. Rejected for now.
- **Defer comments to a separate preprocessing pass**. Strictly
  cleaner but requires either a pre-pass or a peek-driven token
  filter; the in-lexer skip is a half-dozen lines and integrates
  naturally with the existing whitespace handling.
- **Long-bracket comments `--[[ ... ]]` in the same phase**.
  Shares logic with long-bracket strings (`[[...]]`) which we
  don't have yet. Rejected to keep this phase trivially small.

## Consequences

- Lexer gains a six-line comment-skip block right after the
  whitespace skip. No new token, no new error.
- Four lexer-unit tests cover EOF / `\n` / inline / minus-vs-comment.
- Seven integration tests exercise comments at the top of file,
  inline after a call, in the middle of an expression, between
  statements, inside a function body, in a comment-only file, and
  the trailing-newline case.

## Out of Scope

- **Long-bracket comments `--[[ ... ]]`**.
- **Doc-comment conventions** (e.g. `---` for LDoc/EmmyLua).
- **Comment-preserving AST** for formatters / refactoring tools.
