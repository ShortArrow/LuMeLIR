# 0129. Phase Tag Convention

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

ADRs 0007 through 0119 carry "phase tags" like `2.7r-stdlib-table`, `2.devinfra-emit`, `2.8e-iter-ipairs`. These tags emerged organically during Phase 2 build-out; commit `097b885` first attempted to document them in `docs/design/README.md`. The convention was implicit and inconsistent across the early ADRs (some had no phase tag, some had two).

After commit `ef182e0` removed the `AGENTS.md` per-ADR progress table — the original *consumer* of phase tags — the role of phase tags needs to be redefined: they are no longer load-bearing for navigation, but they remain useful metadata on Feature Memo ADRs to keep them grouped by lane.

This ADR formalizes the policy.

## Decision

- **ADR ID is chronological.** The next ADR always takes the next free integer. Dense numbering (no gaps) is enforced by `tests/adr_doc_consistency.rs::adr_doc_numbering_is_dense`.
- **Phase tag is semantic** — describes which lane the decision belongs to (`2.X[a-z]-domain-feature` for feature lanes, `2.devinfra-*` for cross-cutting infrastructure).
- **ADR ID and phase tag are not 1:1.** A single phase lane can contain many ADRs (e.g. `2.7r-stdlib-table` covers 0106 / 0107 / 0108 / 0111 / 0118).
- **Foundational ADRs (0120–0131) have no phase tag.** They are phase-orthogonal — a policy ADR like 0122 (TDD) applies to every phase.
- **Feature Memo and Refactor Memo ADRs retain their phase tag** in the title — they're historical records and the tag preserves the lane grouping that motivated them.
- **New `Architecture Decision`-kind ADRs after 0131 may or may not have a phase tag**, at the author's discretion. If the decision is phase-scoped (e.g. "Phase 3 interop FFI strategy"), include the tag; if cross-cutting (e.g. "Switch to MLIR 23"), omit it.

### Phase tag taxonomy (historical)

The phase tags below appear in ADRs 0007–0119. New ADRs may extend the taxonomy by adding to `docs/design/README.md`.

- **`2.X[a-z]-domain-feature`** — Phase 2 feature lane. `X` is a Phase 2 subphase (0–9), letters extend within. Examples: `2.6a-arr-*`, `2.7q-stdlib-math`, `2.7r-stdlib-table`, `2.7x-stdlib-io`, `2.8e-iter-*`.
- **`2.devinfra-*`** — cross-cutting Tidy First / dev-infrastructure. Examples: `2.devinfra-emit` (ADR 0090), `2.devinfra-stdout-fwrite` (ADR 0117).
- **`2.8f-cli-*`** — CLI surface (arg table, run modes).

## Alternatives considered

- **ADR ID encodes phase** (e.g. `P2-1`, `P2-2`, `P3-1`). Rejected. Loses the dense-numbering invariant; cross-phase decisions (ADR 0083 closures spans 2.5c-* tags and architecture concerns) become hard to ID.
- **Drop phase tags entirely.** Rejected. The existing 119 Feature Memo titles include them; removing would either require renaming all those files (`mv` + git rename churn) or leaving them as orphan strings. Keeping the tag on memos preserves history.
- **Phase tags only on Feature Memo, never on Architecture Decision.** Rejected as too strict. A phase-scoped Architecture Decision (e.g. ADR 0083 on closures) usefully signals "this is a Phase 2 closure-related decision" — readers benefit.

## Consequences

**Positive**
- Future readers can re-derive the lane grouping from `docs/design/README.md`'s taxonomy section without consulting the now-removed AGENTS.md.
- ADR IDs stay simple (monotonic integers).
- Foundational ADRs (0120-0131) being phase-orthogonal is a positive signal — they are *about* the project's invariants, not a specific phase.

**Negative**
- Phase tag inconsistency in 0007-0119 (some have one tag, some have two, some have none) is preserved as history. We do not retro-fix.
- New contributors must learn the taxonomy. Mitigation: it's documented in `docs/design/README.md` and the count of tags is small (~10).

**Locked in until superseded**
- ADR ID = chronological + dense is baseline (audit test enforces).
- Phase tag freedom on new ADRs (use if useful, skip if not) is baseline.

## References

- `docs/design/README.md` "ADR ID vs phase tag" + "Phase tag taxonomy" sections — current rules (introduced in commit `097b885`).
- `tests/adr_doc_consistency.rs::adr_doc_numbering_is_dense` — enforces dense ID.
- Commit `ef182e0` — removed AGENTS.md per-ADR rows, freeing phase tags from their original consumer.
