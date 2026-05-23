# 0131. PR Discipline and Code Review

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

`CONTRIBUTING.md` has carried a brief "PR Discipline" section since the early days of the project, but the policy has not been recorded as a discoverable decision. With multi-LLM-agent collaboration (Claude Code, Codex review), and the prospect of human contributors, formalizing PR scope and review expectations is now load-bearing.

This ADR records the policy. The corresponding PR template (`.github/pull_request_template.md`) and reviewer checklist follow in separate PRs.

## Decision

### One PR = one logical change

- If you find yourself writing "and" in the PR title, split the PR.
- A refactor that is a prerequisite for a feature lands in a separate PR (per ADR 0123), or at minimum a separate commit (a `refactor:` commit landing before the `feat:` commit in the same PR is acceptable for small refactors).

### Required content

- **Title:** a short Conventional Commits-style subject (per ADR 0130). The PR title becomes the squash-merge commit subject when used.
- **Description:** explains the *why* (problem, motivation), references the relevant ADR(s), summarizes the *what* (rough diff shape). Reviewer should not need to read the diff to know the goal.
- **ADR reference:** if the PR implements a decision, link the ADR. If the PR makes a decision, the ADR is part of the PR.
- **Test plan:** PRs that change behaviour describe how the change was tested (which `cargo test --test ...` ran green, manual smoke steps for CLI changes). A behaviour-change PR without tests is **not mergeable** (per ADR 0122).

### Branch naming

- `feat/<short-name>` — new feature.
- `fix/<short-name>` — bug fix.
- `docs/<short-name>` — documentation only.
- `refactor/<short-name>` — behaviour-preserving refactor.
- `chore/<short-name>` — maintenance.
- `test/<short-name>` — test-only.
- `ci/<short-name>` — CI / workflow changes.

Branch off `main`. Mirror the Conventional Commits type from ADR 0130.

### Review

- **Pre-merge review is required.** Currently the project is solo + LLM-agent-assisted; the "reviewer" is the project lead doing a final read-through before merge.
- **Codex review** (or equivalent independent LLM review) is the established pattern for non-trivial PRs: get a 6-視点 (non-ad-hoc / TDD / FP / CA / security / structured docs) read before merge.
- When multi-contributor: at least one human reviewer (other than the author) approves before merge.
- Self-merge of trivial changes (typo fixes, README cleanup) is acceptable; behaviour changes are not self-merge eligible without an external review.

### CI must be green

- Per ADR 0124, all gate commands must pass before merge.
- A PR cannot be merged with a failing CI check unless explicitly justified in a PR comment (e.g. known-flaky CI bug being investigated separately).

## Alternatives considered

- **Stacked PRs** (sequence of dependent PRs reviewed together). Rejected for now: GitHub's stacked-PR tooling is weak, and our PR cadence does not require it. Revisit if Phase 3 introduces large multi-layer features that genuinely cannot be split.
- **Trunk-based with no PR review** (direct push to `main`). Rejected: loses the review checkpoint, loses the test plan documentation, loses the ADR-reference checkpoint.
- **Huge PRs** ("ship the whole feature in one PR"). Rejected: review fatigue is the primary bug-introduction vector. Splitting is always available.
- **Codex review optional / never required.** Rejected: independent review is the cheapest insurance against blind spots. Required for non-trivial changes; the lead's judgment decides "non-trivial" until a more formal rule emerges.

## Consequences

**Positive**
- PRs are small and reviewable; bisecting `main` history is feasible.
- Each PR carries its own justification (description + test plan + ADR ref).
- Codex review pattern surfaces architectural concerns before they reach `main`.

**Negative**
- Splitting a feature into multiple PRs is more wall-clock time than landing one big PR. Accepted: the review/bisect cost of large PRs is higher than the integration cost of small ones.
- The PR template (future work) will add some friction for trivial PRs. Mitigation: template is short, and trivial PRs can be self-merged after a quick read.

**Locked in until superseded**
- One PR = one logical change is baseline.
- Conventional Commits-style title + ADR reference + test plan are baseline.
- CI green is baseline.

## Future work

- `.github/pull_request_template.md` — PR template embedding ADR criteria self-test + doc-update checklist + test-included confirmation. Separate PR.
- Reviewer checklist (under `docs/contributing/` or as a comment template) — codifies the 6-視点 review for non-trivial PRs.
- Multi-contributor onboarding doc — when the second human contributor arrives.

## References

- `CONTRIBUTING.md` "PR Discipline" — current rules.
- ADR 0122 (TDD) — "behaviour change without tests is not mergeable".
- ADR 0123 (TidyFirst) — refactor vs feature commit separation.
- ADR 0124 (CI/CD) — gate that must be green.
- ADR 0130 (commit message convention) — informs the PR title format.
- "Codex 6-視点 review" — the multi-perspective review practice referenced in many existing ADR docs.
