# 0047. Phase 2.9c: Strip Byte-Offset Suffix from Error Display

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-05-03
- **Deciders:** ShortArrow

## Context

After ADR 0045 added `path:line:col` prefixes and ADR 0046
added the source-snippet caret, the rendered error became:

```
lumelir: /tmp/err.lua:2:11: hir error: undefined name 'z' at byte offset 22
  | local y = z
  |           ^
```

The trailing `at byte offset 22` duplicates information
already in the prefix (offset 22 → line 2, col 11) and, worse,
gives readers two coordinate systems in one line. Different
variants used different syntaxes — some `at byte offset N`,
some `(offset N)` — adding noise.

ADR 0045's "Out of Scope" carved this out as a follow-up;
this phase clears the noise.

## Decision

### Drop the offset reference from each variant's `Display`

Every `LexError`, `ParseError`, and `HirError` variant has its
`#[error("…")]` annotation rewritten to omit the offset
substring. Examples:

| Before | After |
|---|---|
| `"undefined name '{name}' at byte offset {offset}"` | `"undefined name '{name}'"` |
| `"unexpected character {ch:?} at byte offset {offset}"` | `"unexpected character {ch:?}"` |
| `"closure with upvalues cannot escape via {position} — direct call only (offset {offset})"` | `"closure with upvalues cannot escape via {position} — direct call only"` |
| `"unexpected end of input at byte offset {offset}"` | `"unexpected end of input"` |

The `offset: usize` **field** stays on every variant — it's
still used by `LexError::offset()` / `ParseError::offset()` /
`HirError::offset()` (added in ADR 0045) and by the diag layer
to compute line/col.

### Rendered output

```
lumelir: /tmp/err.lua:2:11: hir error: undefined name 'z'
  | local y = z
  |           ^
```

One coordinate system (line:col), one location source (the
prefix), one source snippet. The error message itself
describes only what went wrong.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | Display strings on `LexError` variants (5 lines)   |
| Parser   | Display strings on `ParseError::UnexpectedEof` and `UnexpectedToken` |
| AST      | None                                                |
| HIR      | Display strings on `HirError` variants (10 lines) |
| Codegen  | None                                                |
| CLI      | None                                                |

The `offset: usize` field on every variant is preserved so
the `offset()` accessors keep working. No data structure
shape changes.

## TDD Process

1. **Tidy First — by design.** This *is* the cleanup phase.
   Behaviour preserved: every test still passes, and no error
   case stops being detected. Only the rendered string
   shortens.
2. **Red.** The Phase 2.9a regression test
   `parse_error_renders_with_line_col` is strengthened: it
   now asserts `!stderr.contains("byte offset")` *and*
   `!stderr.contains("(offset ")`. Pre-cleanup, this fails.
3. **Green.** All `#[error("…")]` annotations rewritten.
   Strengthened test passes; total green at 622 (no count
   change — pure cosmetic refactor).
4. **Refactor.** None warranted — the change *is* the
   refactor.

## Alternatives Considered

- **Keep the offset, change to a unified syntax** (e.g.
  always `(offset N)`). Less noisy than the current mix,
  but still duplicates the prefix's information.
- **Render only one of `byte offset` or `line:col` based on
  a flag** (e.g. `--byte-offsets` for editor scripts). Real
  use case but premature; nothing currently asks for it.
- **Add the source snippet to library-layer errors** (so
  `format!("{e}")` gives the rich rendering even without the
  CLI). Library users can query offsets and call into
  `cli::diag` themselves; baking presentation into errors
  blurs the layer boundary.

## Consequences

- 17 `#[error("…")]` annotations rewritten across three
  files; offset fields unchanged.
- Strengthened diagnostic e2e test pins the new shape; total
  green at 622.
- ADR 0045's "Out of Scope: Removing the redundant byte
  offset suffix" item retires.

## Out of Scope

- **Source-line context** (prev/next lines around the
  caret).
- **Color output** for TTYs.
- **Tab expansion** in the source snippet.
- **Multi-error reporting** — one error stops compilation.
