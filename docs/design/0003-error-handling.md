# 0003. Error Handling: `thiserror` in Library, `anyhow` at CLI Boundary

- **Status:** Accepted
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

## Context

AGENTS.md §4.4 listed error-handling strategy as TBD. Phase 0 used `Result<(), Box<dyn Error>>` as a stopgap. Phase 1 introduces real library code (lexer, soon parser) whose failures must be:

- **Typed** at module boundaries so callers can `match` on specific cases (e.g. "unexpected character" vs "unterminated string")
- **Source-span-aware** so MLIR diagnostics and CLI pretty-printing can localize the error
- **Ergonomic to propagate** through `?` without `Box`ing everywhere
- **Cheap to extend** when new error cases arise (lexer → parser → HIR → codegen)

The Rust ecosystem has settled on two idiomatic tools:
- **`thiserror`**: derive macro for defining structured error enums (library-friendly, preserves type information).
- **`anyhow`**: opaque `anyhow::Error` that erases type in exchange for ergonomics (application-friendly, great for main/CLI).

## Decision

**Library layers use `thiserror` to define per-layer error enums. The CLI layer (and only the CLI layer) may use `anyhow` to collapse heterogeneous errors for top-level reporting.**

### Per-layer error types

Each inner layer owns its error enum:

```rust
// src/lexer/error.rs
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character {ch:?} at byte {offset}")]
    Unexpected { ch: char, offset: usize },
    #[error("unterminated string starting at byte {start}")]
    UnterminatedString { start: usize },
    // ... extended per phase
}
```

Later layers (`parser`, `hir`, ...) will define their own `*Error` enums and convert inward ones via `#[from]`:

```rust
#[derive(Debug, Error)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] LexError),
    #[error("unexpected token {0:?}")]
    UnexpectedToken(/* ... */),
}
```

### CLI boundary uses `anyhow`

```rust
// src/cli/mod.rs
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Compile { .. } => compile::invoke(/* ... */)?,
        Commands::Run { .. } => run::invoke(/* ... */)?,
    }
    Ok(())
}
```

CLI subcommands may use `anyhow::Context::context` to attach human-readable context at I/O boundaries (e.g. "failed to read {path}").

### Rules

1. **Library code (`src/lexer/`, `src/parser/`, ...) must not depend on `anyhow`.** Check at review time; `cargo tree -p lumelir --edges features` should show `anyhow` only in the `cli` path (or as a dev-dependency).
2. **Library errors must implement `std::error::Error` + `Debug` + `Display`.** `thiserror` does this by default; don't hand-roll.
3. **No `unwrap`/`expect` in non-test library code** (see AGENTS.md §4.4). Tests may unwrap freely.
4. **Span information (`Span { start, end }`) is carried in error variants** so callers can produce diagnostics.
5. **Spans are byte offsets into the original source**, not UTF-8 char indices. Converted to line/column only at the presentation layer.

## Alternatives Considered

### `anyhow` everywhere
Rejected. Erases types, so the parser cannot `match` on specific `LexError` variants for recovery. Fine for end-user binaries but unacceptable for a compiler.

### Hand-rolled error enums without `thiserror`
Rejected. `thiserror`'s `#[error("...")]` and `#[from]` remove boilerplate without hiding behavior. The resulting enum is plain Rust — we can drop the derive later if the crate ever needs to shed a dependency.

### `snafu`
Rejected. Similar space to `thiserror` with richer context-attachment story, but the community standard is `thiserror`, and we don't yet need `snafu`'s selector pattern.

### `miette` (diagnostics-first error crate)
Deferred. `miette` produces beautiful rustc-style reports, which is attractive for a compiler. Revisit once the parser lands and we have multi-span diagnostics to show; adopting it earlier couples error *definitions* to *presentation* prematurely.

## Consequences

**Positive**
- Clear split: inner layers typed, outer layer ergonomic.
- Pattern-matching on specific error variants is preserved where it matters (parser recovery, HIR lowering decisions).
- CLI messages stay readable via `anyhow::Context`.
- Adding a new layer only requires defining one enum + one `#[from]` chain.

**Negative**
- Two error crates instead of one. Mitigated by the strict layering rule — contributors won't mix them by accident.
- `thiserror` adds a proc-macro build-time cost. Acceptable.

**Locked in until superseded**
- Adopting `miette` later would wrap existing `thiserror` types, not replace them — this ADR is forward-compatible.
- If we ever expose `lumelir` as a public library crate, consumers get stable, typed errors from day one.
