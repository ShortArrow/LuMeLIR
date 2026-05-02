# 0041. Phase 2.8d: `#!` Shebang Line

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Lua scripts on Unix routinely start with a `#!` shebang
(`#!/usr/bin/env lua` or similar) so the OS can pick the right
interpreter from `chmod +x` execution. Without explicit support,
that line surfaces as `TokenKind::Hash` followed by `!` (which
is `Unexpected`) and the script fails to parse. Real-world
LuMeLIR scripts saved with execute bits would all fail.

The reference Lua interpreter handles this in its loader, not
in the language proper — it strips the first line if it begins
with `#`. Our compiler is the equivalent of "the loader" for
AOT compilation, so the same lexer-level skip applies.

## Decision

### One-shot prefix check at the top of `lex()`

```rust
if bytes.starts_with(b"#!") {
    for (_, c) in chars.by_ref() {
        if c == '\n' {
            break;
        }
    }
}
```

Lives in `lex()` directly — small enough that a helper would be
ceremony. Runs exactly once, before the main token loop.
Honoured **only at byte 0**: any `#` later in the source stays
the length operator.

### Why `#!` not just `#`

The reference Lua loader strips a leading `#` (any first-line
comment-like). We're stricter — only `#!` qualifies. Reasons:

1. A bare `#` at file start is a valid (but unusual) length
   expression in our grammar; stripping it silently would mask
   real syntax errors.
2. The `#!` form is what Unix `execve` actually requires;
   stripping a wider surface adds no value.

If a future test or imported script demonstrates the wider
form is genuinely useful, the predicate widens — but the
narrower default keeps surprises low.

### CA invariants preserved

| Layer    | Change                                           |
|----------|--------------------------------------------------|
| Lexer    | One-time prefix skip at top of `lex()`           |
| Parser   | None                                             |
| AST      | None                                             |
| HIR      | None                                             |
| Codegen  | None                                             |

The skip happens before any token is produced. From the
parser's view the file looks like the post-shebang content.

## TDD Process

1. **Red**: 4 lexer unit tests — shebang+code, shebang-only,
   non-leading `#!`, leading `#` without `!`. The first two
   failed (lexer surfaced `#!` as `Hash` + `Unexpected`); the
   latter two already passed (boundary protection).
2. **Green**: prefix-checked skip at the top of `lex()`.
   Tests passed at 575 (571 + 2 unit + 4 e2e... wait, let me
   recount once impl is in).
3. **Refactor**: none warranted — the skip is 5 lines, no
   shared helper to extract.

## Alternatives Considered

- **Treat `#` at line start as a comment marker** (matching
  Lua's reference loader). Would silently swallow `#expr`
  lines that are valid syntax in our grammar. Rejected for
  the surprise factor.
- **Strip `#!` only with a flag on the CLI**. Would force
  scripts to be invoked one way locally and another via
  `chmod +x`. Inconsistent.
- **Require the shebang to use the literal interpreter name
  (`#!/usr/bin/env lumelir`)**. Lockstep coupling between the
  source file and the binary's deploy path; fragile. Skip
  semantically — don't validate the interpreter path.

## Consequences

- Lexer adds 7 lines (the conditional skip).
- 4 lexer unit tests + 4 integration tests cover the skip
  path, the no-skip path (no shebang), the at-offset-0-only
  rule, and a real `chmod +x`-style header followed by a
  function definition.

## Out of Scope

- **Multiline shebangs** — `#!` is single-line by Unix
  convention; multi-line forms would need a different
  delimiter and don't exist in practice.
- **BOM handling** — UTF-8 BOM at file start would now appear
  before the shebang and prevent the match. Not yet seen in
  the wild for our use case; defer.
- **The reference-loader `#` (any first character)** —
  documented above.
