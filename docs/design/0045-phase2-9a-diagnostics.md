# 0045. Phase 2.9a: Line/Column Diagnostics at the CLI Boundary

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Lower layers (lexer, parser, HIR) attach **byte offsets** to
their error variants — a stable, encoding-agnostic anchor that
suits internal use. ADR 0003 settled on this choice for the
library API.

The CLI rendered errors as bare `Display` strings:

```
lumelir: hir error: undefined name 'z' at byte offset 22
```

Byte offsets aren't actionable for users — nobody scrolls
through source counting bytes. Editors and other tools speak
`path:line:col:` (rustc, gcc, clippy, the LSP convention).

## Decision

### Pure helper: `offset_to_line_col`

```rust
pub fn offset_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(src.len());
    let prefix = &src[..clamped];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = src[line_start..clamped].chars().count() + 1;
    (line, col)
}
```

1-based line, 1-based column counting Unicode scalar values
(matches rustc rendering and editor expectations). Out-of-range
offsets clamp to `src.len()` so error rendering never panics.
Pure relative to its inputs.

### `HasOffset` trait + impls

A small trait abstracts over the lexer / parser / HIR error
enums:

```rust
pub trait HasOffset {
    fn offset(&self) -> usize;
}
```

Each enum gets an inherent `pub fn offset(&self) -> usize` that
matches all variants (every variant carries an offset by ADR
0003 convention). The diag module impls `HasOffset` by
delegating to those inherent methods, keeping the trait
boundary in the CLI layer where it belongs (lower layers don't
care about presentation).

### CLI integration: `format_error`

```rust
pub fn format_error<E: HasOffset + Display>(
    src: &str, path: &Path, layer: &str, err: &E,
) -> String {
    let (line, col) = offset_to_line_col(src, err.offset());
    format!("{}:{}:{}: {} error: {}", path.display(), line, col, layer, err)
}
```

`compile.rs` and `run.rs` route every Lex/Parse/Hir error
through this formatter:

```rust
.map_err(|e| anyhow::anyhow!("{}", diag::format_error(&source, input, "parse", &e)))
```

The user sees:

```
lumelir: /tmp/err.lua:2:11: hir error: undefined name 'z' at byte offset 22
```

instead of:

```
lumelir: hir error: undefined name 'z' at byte offset 22
```

### CA invariants preserved

| Layer    | Change                                                  |
|----------|---------------------------------------------------------|
| Lexer    | Inherent `LexError::offset()` method (no Display change) |
| Parser   | Inherent `ParseError::offset()` (delegates to LexError on `Lex` arm) |
| AST      | None                                                    |
| HIR      | Inherent `HirError::offset()` method                    |
| Codegen  | None — codegen errors don't carry source offsets (operate on validated HIR) |
| CLI      | New `cli::diag` module (helper + trait + impls); `compile.rs` and `run.rs` call `format_error` |

The dependency direction is preserved: the trait is **defined
in the CLI layer**, with impls also in the CLI layer (referring
inward to lower-layer types). Lower layers know nothing about
presentation.

## TDD Process

1. **Red.**
   - 8 unit tests for `offset_to_line_col` covering offset 0,
     end-of-line, multi-line, character-vs-byte counting,
     empty source, out-of-range, and the trailing-newline case.
   - 3 e2e tests invoking the `lumelir compile` binary on
     intentionally bad sources and asserting on the rendered
     stderr format (`:line:col:` prefix and layer tag).
   The unit tests passed immediately (the helper was written
   alongside them, no harness gap). The e2e tests failed
   because the CLI was still using the old format string.
2. **Green.** Added inherent `offset()` methods on the three
   error enums; added `cli::diag` module with the helper,
   trait, impls, and formatter; rewrote the two `map_err`
   sites in `compile.rs` and `run.rs`. All e2e tests passed
   at 615 (604 + 11).
3. **Refactor.** None warranted — the new module is small
   and the CLI integration is two lines per command.

## Alternatives Considered

- **Strip `"at byte offset N"` from each variant's Display
  string** so the rendered error doesn't double up. Real
  improvement, but mechanical (every variant in three enums
  needs its `#[error(...)]` annotation rewritten). Defer to
  a follow-up if the duplication becomes noisy in practice.
- **Render with column counting bytes** instead of chars.
  Faster (no `chars().count()`), but drops the rustc/LSP
  convention. The performance difference is irrelevant for
  diagnostic rendering paths.
- **Define `HasOffset` in a shared util crate** so multiple
  layers can implement it directly. Currently only the CLI
  layer cares; introducing a util crate for one trait is
  premature.
- **Use `codespan-reporting` or similar**. Pulls in a heavy
  dependency for a feature with one use case. Defer until
  we want span-aware multi-line snippet rendering.

## Consequences

- New module `src/cli/diag.rs` (~80 lines incl. tests).
- Three inherent `offset()` methods (~30 lines net across
  the three error files).
- 8 unit tests + 3 e2e tests; total green at 615.
- Output format: `path:line:col: <layer> error: <message>`.

## Out of Scope

- **Source-snippet rendering** (the rustc-style "→ <line content>
  → ^^^^^ here" caret display). Real value but its own ADR.
- **Removing the redundant `byte offset N` suffix** in error
  variant Display strings.
- **Absolute path resolution** — the rendered path is whatever
  the user passed on the command line (relative or absolute).
- **Multi-error reporting** — one error stops compilation.
  Multi-error mode is its own design problem.
