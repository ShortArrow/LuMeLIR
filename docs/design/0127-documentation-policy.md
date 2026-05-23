# 0127. Documentation Policy

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

The repository's documentation has gone through several iterations as the project grew:

- Phase 0 had a single `AGENTS.md` mixing conventions, progress, and TODO.
- Phase 2 saw `AGENTS.md` grow past 350 lines with per-ADR progress rows.
- Commits `ed34703` (AGENTS.md row slim), `097b885` (ADR docs cleanup), and `ef182e0` (AGENTS → LLM-safety-only) progressively split responsibilities across files.

This ADR codifies the final responsibility split so the same drift does not recur.

## Decision

### File responsibilities

| File | Role | Mutability |
|---|---|---|
| `docs/PRD.jp.md` | **Product Requirements (SoT, Japanese).** Phases, goals, scope. | Mutable as product evolves; commit changes with corresponding ADR if architectural. |
| `docs/PRD.md` | Best-effort English translation of PRD.jp.md. **Drift is acknowledged**; footer points back to the Japanese SoT. | Mutable; updates lag jp version. |
| `README.md` | Project overview (English, primary). | Mutable. |
| `docs/README.jp.md` | Japanese translation of README. | Mutable; translation. |
| `CONTRIBUTING.md` | Universal working conventions for humans + LLMs (FP / CA / TDD / TidyFirst / commits / PR / deps / docs / setup). | Mutable as practice evolves. |
| `AGENTS.md` | LLM-agent safety guardrails only (destructive ops, do-not-touch, commit/push instructions, ask-when-in-doubt). | Mutable for safety rules only — no conventions, no progress, no TODO. |
| `CLAUDE.md` | Thin pointer: "conventions → CONTRIBUTING.md; LLM safety → AGENTS.md". | Update when CONTRIBUTING/AGENTS shape changes. |
| `docs/design/README.md` | ADR conventions + chronological index. | Mutable as conventions evolve. |
| `docs/design/NNNN-*.md` | Canonical single-decision records. | **Immutable once accepted**; supersede via new ADR + `Superseded by` header. |
| `docs/design/tagged-semantics.md` | Phase 2.6c TaggedValue SoT (slot layout, producer/consumer matrix). | Mutable; ADR 0068 records its role. |

### Update discipline

- **Doc update lands in the same PR as the code/ADR change it documents.** Stale docs are the worst failure mode — they look authoritative while being wrong.
- **Cross-references resolve.** Every `(ADR NNNN)` mention in `CONTRIBUTING.md` / `AGENTS.md` / docs points at an existing ADR doc. CI enforces this (see `tests/adr_doc_consistency.rs`).
- **Universal conventions live in `CONTRIBUTING.md`, not `AGENTS.md`.** LLM-specific guardrails (and only those) live in `AGENTS.md`.
- **Roadmaps / TODO / phase status live in `docs/PRD.jp.md` (or external trackers)**, not `AGENTS.md` and not `CONTRIBUTING.md`. Per-ADR chronological detail lives in `docs/design/README.md`.

## Alternatives considered

- **Single mega-`AGENTS.md`** (the pre-`ef182e0` state). Rejected. Multiple distinct audiences (humans, LLM agents, future-self) need different framings; bundling them into one file produced a 350-line wall that nobody read cover-to-cover.
- **English PRD as SoT.** Rejected. The project lead works in Japanese; forcing English-primary creates drift in the *wrong direction* (the SoT diverges from the lead's intent).
- **No `CLAUDE.md`** (let Claude Code read AGENTS.md directly). Rejected. `CLAUDE.md` is the convention Claude Code expects; a 10-line pointer is the cleanest satisfaction of that contract.
- **Per-component READMEs** (e.g. `src/codegen/README.md`, `src/hir/README.md`). Rejected as navigation-fragmenting. Module-level rationale belongs in ADRs; module-level usage belongs in `cargo doc`-style rustdoc comments.

## Consequences

**Positive**
- Each file has one audience and one purpose; readers know where to look.
- `AGENTS.md` is small enough to read in full before every LLM-agent session.
- `CONTRIBUTING.md` is small enough for humans to read on PR onboarding.
- Stale-doc failures are detectable (cross-reference resolution).

**Negative**
- More files to maintain (12 listed above). Mitigation: most are stable; only `CONTRIBUTING.md`, the active ADR doc, and `tagged-semantics.md` see frequent updates.
- Same content sometimes belongs in multiple places (e.g. "TDD is the rule" in CONTRIBUTING + foundational ADR 0122). We accept brief duplication where the *framing* differs (current rule vs decision rationale) and avoid duplication of the *same* framing.

**Locked in until superseded**
- The 11-file responsibility split is baseline. Moving content between files requires updating cross-references in the same PR.

## References

- `CONTRIBUTING.md` §8 "Documentation Update Policy" — current rules.
- Commits `ed34703`, `097b885`, `ef182e0` — the responsibility-split journey.
- ADR 0068 (tagged-semantics SoT) — the model for "SoT pointer ADR".
- ADR 0125 (release procedure) — defines `CHANGELOG.md` placement.
