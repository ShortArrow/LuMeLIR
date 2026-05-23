# 0121. Layering: Clean Architecture

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

ADR 0002 established `lib.rs` + `main.rs` and named the dependency direction `cli → lib → codegen → mir → hir → parser → lexer` in passing, but did not formalize it as a standalone policy. As the codebase grew (codegen now ~7000 LOC, hir ~5000 LOC), several near-misses appeared where a lower layer was tempted to call into an upper one (codegen wanting to "re-lower" a string via parser; hir tempted to call into codegen helpers).

This ADR makes the layering rule explicit as its own policy, separately from the lib.rs scaffolding decision.

## Decision

Dependency direction is **strictly inward**:

```
cli  →  (lib crate root)  →  codegen  →  mir  →  hir  →  parser  →  lexer
```

- Each layer may `use` items only from layers strictly inside it.
- Reverse dependencies are forbidden at compile time (caught by `use` path resolution).
- MLIR / Melior / LLVM-sys bindings are confined to `codegen`. `parser`, `hir`, `mir` use plain Rust types (`String`, `Vec`, custom enums) and never see `melior::ir::*`.
- `cli` is the only layer permitted to perform user-facing I/O effects (read files, spawn `mlir-opt` / `mlir-translate`, write binaries).

Cross-layer error conversion happens at each boundary via `From` / `?` (see ADR 0003).

## Alternatives considered

- **Flat module structure** (`src/lib.rs` declares `mod lexer; mod parser; mod hir;` all at the same depth, no enforced direction). Rejected. Phase 2 codegen had a near-miss where a helper wanted to call `parser::parse_string_escape` from inside an MLIR-shape decision. With a flat structure, the compiler does not catch that mistake.
- **Put `mlir` at the top level alongside `cli`** (so `mlir::*` re-exports are visible everywhere). Rejected. ADR 0005 confirmed WSL2-only MLIR ABI; making MLIR types appear in lexer/parser/hir would force every layer to handle the WSL2 conditional compile. `codegen` is the right containment.
- **Workspace split** (`lumelir-frontend` crate + `lumelir-codegen` crate). Rejected for now. Cargo workspaces add ceremony with no current payoff; revisit when Phase 3's Rust interop introduces a runtime crate.

## Consequences

**Positive**
- A wrong-direction `use` is a compile error, not a code-review catch.
- `parser` / `hir` / `mir` build on any host (Windows pure-Rust included); only `codegen` requires the MLIR toolchain.
- Integration tests under `tests/` import via `lumelir::...`, which forces them through the public crate surface — they cannot accidentally couple to internal layering.

**Negative**
- Adding helpers that *seem* to want to span layers (e.g. a string-escape helper used by both parser and codegen) must live in the inner layer with a public re-export — minor extra `pub use` ceremony.
- Cross-layer test fixtures (an end-to-end test that runs lexer → parser → hir → codegen) must live under `tests/` (integration), not in any single module's `#[cfg(test)]`.

**Locked in until superseded**
- Removing or reordering this layering requires a successor ADR.

## References

- `CONTRIBUTING.md` §3.2 "Clean Architecture (Layering)" — current rules.
- ADR 0002 (lib-rs layering) — established the root `lib.rs` + thin `main.rs` shape; this ADR formalizes the directional rule.
- ADR 0005 (MLIR environment) — explains why MLIR types belong in `codegen`.
- ADR 0120 (FP-first) — pairs naturally: effect at codegen boundary is the same boundary as the layering rule.
