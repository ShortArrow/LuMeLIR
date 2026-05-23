# 0123. TidyFirst and Refactor Discipline

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

Phase 2 demonstrated several cases where adding a new feature required reshaping existing code first — e.g. ADR 0091 (callee normalization) was a prerequisite for ADR 0092 (method colon syntax); ADR 0073 (codegen module split) was a prerequisite for further tagged-value work. In each case the cost of "fix while you're there" — interleaving the refactor into the feature commit — would have produced a PR whose diff was hard to review and hard to roll back.

We have been following Kent Beck's *Tidy First?* practice (refactor before adding the change; refactor in its own commit) without recording the policy. This ADR codifies it.

## Decision

- **Refactor commits stay separate from feature commits.** A refactor that is a prerequisite for a feature lands in its own commit (or its own PR), tested green, before the feature commit.
- **`refactor:` Conventional Commits prefix** (see ADR 0130) signals "behaviour unchanged, structure improved". The test corpus before and after a `refactor:` commit must be identical (count and outcome).
- **Rule of three before extracting a helper.** Do not extract a helper from a single call site, or from two near-identical call sites. Wait until three sites want the same shape — then the abstraction has earned its keep.
- **Tidy → then change.** When a feature needs a refactor, the order is: (1) refactor commit landing the prerequisite shape, (2) feature commit using the new shape. The reverse — "land the feature with both refactor and behaviour change" — is rejected.

## Alternatives considered

- **"Fix while you're there"** (interleave refactor and feature in one commit). Rejected. Review noise drowns the feature; if the refactor turns out to break something, the feature diff is harder to bisect / revert.
- **"No refactor without a bug"** (only touch code when fixing). Rejected. Tech debt accumulates; eventually a feature that should be small becomes large because the surrounding code is wrong shape. The rule-of-three threshold gives a clear trigger to refactor without inventing a bug to justify it.
- **Refactor anytime, no separation rule.** Rejected. Without a clear "refactor commit ≠ feature commit" rule, every PR risks bundling shape changes with behaviour changes — which breaks the test-as-safety-net guarantee from ADR 0122.

## Consequences

**Positive**
- A `refactor:` commit always means "test corpus unchanged" — a strong invariant for bisecting.
- Feature commits keep their diffs focused on the actual feature.
- Rule of three prevents premature abstraction (the most common form of accidental complexity in a young codebase).

**Negative**
- A feature that needs significant refactor produces 2-3 commits where one might feel "simpler". We accept this — the alternative produces unreviewable diffs.
- Some refactors are not strictly behaviour-preserving in performance (e.g. moving a hot loop into a helper). For these we note the perf delta in the commit message; if the delta matters, a benchmark goes in `tests/`.

**Locked in until superseded**
- The refactor/feature separation rule is the baseline. A future ADR could refine the rule for performance-sensitive refactors, but reverting to "interleave freely" requires explicit deprecation of this ADR.

## References

- `CONTRIBUTING.md` §3.4 "Rust-Specific Guidance" — the rule-of-three line.
- ADR 0122 (TDD) — pairs naturally: refactor commits depend on the green test corpus as their safety net.
- ADR 0130 (commit message convention) — defines the `refactor:` prefix.
- Kent Beck, *Tidy First?*.
- ADR 0073 (codegen module split) and ADR 0091 (callee normalization) — examples of refactor-as-prerequisite in this codebase.
