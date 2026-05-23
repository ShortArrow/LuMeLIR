# 0124. CI/CD Policy

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

Until now there has been no CI configuration in the repository. Developers run the local gate (`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`) on each PR, but there is no automated enforcement and no shared signal for "is `main` green". As contributor count grows (currently solo, multi-LLM-agent collaboration), automation is needed.

This ADR records the CI/CD decision; the actual workflow file (`.github/workflows/ci.yml`) is created in a follow-up PR per ADR 0128 (dependency policy — workflow file is configuration, not code, but the principle of "land the policy first, the implementation second" applies).

## Decision

- **CI provider: GitHub Actions.** Workflow files live at `.github/workflows/*.yml`.
- **Triggers:**
  - On pull request to `main`.
  - On push to `main` (post-merge sanity).
- **Gate (must pass for PR merge):**
  ```
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```
- **MLIR-linked test execution** requires an environment with MLIR 22 (matching melior 0.27, per ADR 0005). Two acceptable approaches:
  - **Self-hosted runner** on a WSL2 Arch host with the toolchain pre-installed (low setup cost for the project lead; not portable to forks).
  - **Docker image with MLIR 22 prebuilt** (portable; future PR will publish it under `ghcr.io/<owner>/lumelir-ci`).
  The decision between the two is deferred to the workflow-implementation PR; both satisfy this policy.
- **Release artifacts** are out of scope for this ADR — see ADR 0125.

## Alternatives considered

- **GitLab CI.** Rejected: repository is hosted on GitHub; switching CI provider would require also switching forge.
- **Self-hosted Jenkins.** Rejected: setup, maintenance, and credential management costs are disproportionate for a small project.
- **No CI** (continue relying on local-gate discipline). Rejected: PR reviewers cannot trust a "I ran fmt/clippy/test locally" claim without an automated check; multi-LLM-agent collaboration in particular needs an oracle outside any single agent.
- **Run only `cargo test` in CI** (skip fmt + clippy as too cheap to bother automating). Rejected: a clippy warning slipping into `main` is exactly the kind of regression CI exists to prevent.

## Consequences

**Positive**
- `main` carries a verifiable green signal.
- PR review starts from "CI is green, focus on the diff" rather than "did you run the gate?".
- Future PR templates can require a CI-green check before merge.

**Negative**
- CI time is dominated by MLIR build cost (~minutes). Caching strategies (`Swatinem/rust-cache`, MLIR artifact caching) are future work.
- Self-hosted runner introduces an availability dependency on the lead's WSL2 host. Switching to docker is the mitigation.
- Workflow file maintenance becomes a doc-update obligation when the gate changes.

**Locked in until superseded**
- "GitHub Actions" as provider is locked. Switching providers requires a new ADR.
- The 3-command gate is the minimum; additional gates (`cargo audit`, `cargo deny`, etc.) are added incrementally as their respective policy ADRs land.

## Future work

- `.github/workflows/ci.yml` — actual workflow file. Follow-up PR.
- `cargo audit` CI step — see ADR 0126.
- Release artifact generation — see ADR 0125.
- CI run-time optimization (rust-cache, MLIR prebuilt image).

## References

- `CONTRIBUTING.md` "Local Gate" — the same command sequence that CI mirrors.
- ADR 0005 (MLIR environment) — explains why CI needs an MLIR 22 environment.
- ADR 0122 (TDD) — the test corpus that CI runs.
- ADR 0128 (dependency policy) — workflow files added per-need.
