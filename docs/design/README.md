# LuMeLIR Design Notes

This directory collects **Architecture Decision Records (ADR)** for LuMeLIR.
Each file captures a single design decision — what was chosen, what was rejected, and why.

A top-level `DESIGN.md` may appear once enough ADRs exist to warrant an index; until then, ADRs are the canonical design documentation.

## Conventions

- Filename: `NNNN-kebab-title.md` (zero-padded, monotonically increasing — e.g. `0001-parser-choice.md`)
- One decision per file. If a later ADR supersedes an earlier one, add a `Supersedes:` / `Superseded-by:` header rather than editing history
- Write in the present tense of the decision ("We choose X"), not future plans ("We will decide X")

## ADR Template

```markdown
# NNNN. <Title>

- **Status:** Proposed | Accepted | Superseded by NNNN | Deprecated
- **Date:** YYYY-MM-DD
- **Deciders:** <names / handles>

## Context

What problem forced a decision? What constraints apply?

## Decision

The choice made, stated plainly.

## Alternatives Considered

Each rejected option + the reason it was rejected.

## Consequences

What becomes easier, harder, or locked-in as a result.

## Documentation updates

`docs/design/tagged-semantics.md` is the SoT for the
TaggedValue runtime model (ADR 0068). Any ADR that touches a
TaggedValue producer, consumer, dispatch site, or runtime
invariant **must** record updates as a checklist below:

- [ ] §1 slot layout
- [ ] §2 producer / source taxonomy
- [ ] §3 consumer coverage matrix
- [ ] §4 LIC consolidation
- [ ] §5 runtime tag invariants
- [ ] §7 open questions
- [ ] §8 ADR index
- [ ] No `tagged-semantics.md` change required (briefly justify why)
```

## Index

- [0001 — Lexer implementation: hand-written](0001-lexer-implementation.md)
- [0002 — Split into `lib.rs` + `main.rs` for Clean Architecture layering](0002-lib-rs-layering.md)
- [0003 — Error handling: `thiserror` in library, `anyhow` at CLI boundary](0003-error-handling.md)
- [0004 — Parser implementation: hand-written recursive descent with Pratt](0004-parser-implementation.md)
- [0005 — MLIR integration environment: WSL2 (Arch) primary, Windows native best-effort](0005-mlir-environment.md)
- [0006 — Phase 1 codegen: standard MLIR dialects, no HIR, melior 0.27](0006-phase1-codegen.md)
- [0007 — Phase 2.0: `local` bindings, multi-statement, HIR introduction](0007-phase2-0-local-and-multistmt.md)
