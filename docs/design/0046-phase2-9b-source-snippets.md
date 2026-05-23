# 0046. Phase 2.9b: Source-Snippet Caret Display

- **Status:** Accepted
- **Kind:** Refactor Memo
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.9a (ADR 0045) gave error messages a `path:line:col`
prefix. Useful for editors, but humans reading the terminal
still have to hop back to the source file to see what went
wrong. The well-known rustc/clang shape is to inline the
offending line of source plus a caret marker:

```
/tmp/err.lua:2:11: hir error: undefined name 'z' …
  | local y = z
  |           ^
```

The marker turns a byte coordinate into a visual one, and the
inlined line removes the editor round-trip for quick fixes.

ADR 0045's "Out of Scope" explicitly carved this out as a
follow-up; this phase delivers it.

## Decision

### One pure helper: `snippet`

```rust
pub fn snippet(src: &str, offset: usize) -> String {
    let clamped = offset.min(src.len());
    let line_start = src[..clamped].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = src[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(src.len());
    let line = &src[line_start..line_end];
    let col_chars = src[line_start..clamped].chars().count();
    let mut out = String::new();
    out.push_str("  | ");
    out.push_str(line);
    out.push('\n');
    out.push_str("  | ");
    for _ in 0..col_chars {
        out.push(' ');
    }
    out.push('^');
    out.push('\n');
    out
}
```

Output is exactly two lines plus a trailing newline. The
gutter is a fixed two-space + `|` + space pad that matches
rustc's no-line-number rendering. The line-number isn't
re-emitted because the location header already carries it
one line up.

The caret column reuses `offset_to_line_col`'s convention
(Unicode scalars from the line start), so the caret aligns
visually under the column the header reports.

### `format_error` calls `snippet`

```rust
pub fn format_error<E: HasOffset + Display>(
    src: &str, path: &Path, layer: &str, err: &E,
) -> String {
    let offset = err.offset();
    let (line, col) = offset_to_line_col(src, offset);
    let snip = snippet(src, offset);
    format!("{}:{}:{}: {} error: {}\n{}", path.display(), line, col, layer, err, snip)
}
```

The message stays a single returned `String`, so anyhow's
chain printing and downstream formatters see one block of
text.

### Edge cases nailed in tests

- **Offset at column 1** — caret directly under the first
  character.
- **Offset past EOF** — clamps to EOF; caret lands one past
  the last character of the last line.
- **Empty source** — emits a blank source line and a caret
  at column 1, so callers don't need to special-case zero
  input.
- **Multi-byte characters** — caret column counts Unicode
  scalars, matching the `(line, col)` reported by
  `offset_to_line_col`. The visual width can still drift for
  full-width CJK characters (the caret indents by char
  count, not display width); a future ADR can adopt a
  width-aware display via `unicode-width` if it becomes a
  problem in practice.

### CA invariants preserved

| Layer    | Change                                              |
|----------|-----------------------------------------------------|
| Lexer    | None                                                |
| Parser   | None                                                |
| AST      | None                                                |
| HIR      | None                                                |
| Codegen  | None                                                |
| CLI      | One pure helper added to `cli::diag`; `format_error` extended to call it |

The change is entirely contained in the diagnostic
presentation layer.

## TDD Process

1. **Red.** 6 unit tests covering single-line, first-column,
   multi-line picking, multi-byte alignment, past-EOF clamp,
   and empty-source. 1 e2e test asserting both the source
   line and a column-aligned caret appear in stderr. Compile
   refused (`error[E0425]: cannot find function `snippet`')
   — the canonical "the test names a symbol the impl doesn't
   provide" Red signal.
2. **Green.** Added `snippet`; extended `format_error` to
   append it. Both unit + e2e tests pass at 622 (615 + 7).
3. **Refactor.** None warranted — the helper is small enough
   that its only useful "extract" would be the caret-padding
   loop, which is one-liner. Rule of three.

## Alternatives Considered

- **Use `codespan-reporting`** for snippet rendering. Real
  capability (multi-line spans, color, tab-expansion), but
  pulls in a dependency tree that would dominate the binary
  for a feature that's currently two-line text. Defer.
- **Show context lines (`prev_line` + `target_line` +
  `next_line`).** Useful for syntax errors that span
  reorderings but adds gutter logic for line-number
  alignment. Defer until users ask for it.
- **Use `unicode-width` to align the caret under wide CJK
  characters.** `あ` displays at width 2 in monospace
  terminals but counts as 1 char; the caret lands one column
  early. Not addressed here — see the Out of Scope note.
- **Highlight the bad token with a span (`^^^^^`)** rather
  than a single caret. Requires error variants to carry
  span lengths; currently they carry only an offset. Defer.

## Consequences

- `cli::diag` adds ~30 lines (one helper).
- 6 unit tests + 1 e2e test; total green at 622.
- Default error rendering now spans three lines (header +
  source line + caret line). Users keep their workflow but
  get visual context.

## Out of Scope

- **Tab expansion** — tabs in source render as one column,
  so a leading-tab line will misalign the caret. Real
  failure mode but rare in Lua.
- **East Asian Wide character width** — the caret indents
  by char count, not display width.
- **Color output** — TTY detection + ANSI escapes. Worthy
  but a separate concern.
- **Span-aware `^^^^^` ranges** — pending error variants
  that carry span lengths.
