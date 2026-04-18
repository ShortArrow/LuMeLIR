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
```

## Index

_No ADRs yet. The first one lands when Phase 1 PoC work begins (likely `0001-parser-choice.md` or `0001-melior-vs-direct-ffi.md`)._
