# 0002. Split into `lib.rs` + `main.rs` for Clean Architecture Layering

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

## Context

Phase 0 shipped a single binary crate with one `cli` module. Phase 1 introduces the lexer and prepares for parser / HIR / MIR / codegen layers. To honor AGENTS.md §4.2 (Clean Architecture, dependency direction `cli → lib → codegen → mir → hir → parser → lexer`) we need:

- A unit-testable core independent of the CLI entry point
- A clear boundary for `use` dependencies (outer layers may depend on inner, not vice versa)
- The ability to expose the crate as both a binary (`lumelir`) and a library (for future embedding, integration tests, and `cargo doc`)

Integration tests under `tests/` can only exercise crate public API — so without a library component, we cannot write CLI-level integration tests that call into the compiler programmatically.

## Decision

Introduce `src/lib.rs` as the library root. The binary becomes a thin entry point that delegates to library code.

### Layout

```
src/
├── lib.rs          # pub mod lexer; pub mod cli; (parser/hir/mir/codegen added per phase)
├── main.rs         # <20 lines, calls lumelir::cli::run()
├── lexer/
│   └── mod.rs      # Phase 1 starting point
└── cli/
    ├── mod.rs      # moved from bin-only; re-exported via lib
    ├── compile.rs
    └── run.rs
```

### Cargo.toml changes

```toml
[lib]
name = "lumelir"
path = "src/lib.rs"

[[bin]]
name = "lumelir"
path = "src/main.rs"
```

A package with both `[lib]` and `[[bin]]` of the same name is supported by Cargo; the binary target links the library. This gives us both `use lumelir::lexer;` inside `tests/` and the `lumelir` CLI on disk.

### Layering rules (restated)

1. Each inner module has **no** dependency on outer modules. `lexer` cannot `use` `parser`, etc.
2. External MLIR/LLVM FFI is confined to (future) `codegen`.
3. `cli` is the only layer permitted to perform I/O side effects (read files, write binaries, call processes). Everything inner is pure.
4. Shared error types live at the narrowest layer that needs them; cross-layer errors are converted via `From`/`?` at each boundary (see ADR 0003).

### `main.rs` shape

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    match lumelir::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lumelir: {err}");
            ExitCode::FAILURE
        }
    }
}
```

## Alternatives Considered

### Keep bin-only until Phase 2
Rejected. Without a library crate, integration tests must spawn the binary as a child process, which is slow, flaky, and hides structural mistakes in the layering. The sooner the boundary exists, the less retrofitting later.

### Split into a Cargo workspace (`lumelir-core` + `lumelir-cli`)
Rejected for Phase 1. Workspaces add ceremony (`Cargo.toml` indirection, multiple `target/`s for tools that don't share) with no immediate payoff. Revisit when Phase 3's Rust-Lua bridge or a separate runtime crate arrives.

### Expose everything from `cli` re-exports without a `lib.rs`
Not meaningfully different from the rejected bin-only option. Integration tests still cannot `use lumelir::...`.

## Consequences

**Positive**
- Integration tests under `tests/` can directly call library APIs.
- Layer boundaries become type-checked: attempting to `use` a wrong-direction module fails to compile.
- `cargo doc` produces useful library documentation from day one.
- The CLI entry point stays obviously trivial, which is a good litmus test — if `main.rs` grows beyond ~30 lines, something leaked outward.

**Negative**
- `Cargo.toml` gains a `[lib]` stanza; marginal complexity.
- Public API surface must be curated (what is `pub` vs `pub(crate)`). This is a feature, not a bug — it forces intentional design.

**Locked in until superseded**
- No public API stability promise yet (pre-1.0, Phase 1).
- If a future phase demands separate versioning for core vs CLI, revisit with a workspace ADR.
