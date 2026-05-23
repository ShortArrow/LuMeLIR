# 0125. Release Procedure

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

There has been no release process. The repository has accumulated 130+ commits on `main`; there is no `v*` tag, no `CHANGELOG.md`, no published artifact. For a project that aims to be a usable Lua-to-native compiler (per `docs/PRD.jp.md`), this gap will become a problem as soon as the first user wants to pin a known-working version.

This ADR records the release decision. Actual changelog scaffolding and release scripting follow in separate PRs.

## Decision

- **Versioning:** Semantic Versioning (semver).
  - **Pre-1.0** (current): `v0.Y.Z`. Pre-1.0 explicitly allows breaking changes at any minor bump.
  - **Phase milestones map to minor versions:**
    - Phase 1 (PoC) complete → `v0.0.x` (we are here).
    - Phase 2 (core semantics) complete → `v0.1.0`.
    - Phase 3 (Rust-Lua interop) begins → `v0.2.0`.
    - First stable release → `v1.0.0` (Phase 3 complete + production-ready story).
- **Git tags:** annotated tag `v0.Y.Z` on the release commit. Tag message references the changelog entry.
- **`CHANGELOG.md`** at repo root, [Keep a Changelog](https://keepachangelog.com) format. Each PR adds to the `[Unreleased]` section; release cuts the section into a dated version.
- **Release notes** are the corresponding `CHANGELOG.md` section, surfaced via GitHub Releases.
- **Artifact distribution** is out of scope for v0.0.x. As Phase 2 nears completion, a follow-up ADR will decide between prebuilt binaries / `cargo install` / `crates.io` publish / a runtime image.

## Alternatives considered

- **Calendar Versioning (`v2026.05.0`).** Rejected: a language compiler is consumed by downstream code that expects semver. CalVer makes "is this breaking?" unanswerable without reading the changelog every time.
- **No tags, just rolling `main`.** Rejected: bisection across phase boundaries becomes painful; embedders cannot pin.
- **Immediate `v1.0.0`.** Rejected: API surface (HIR, codegen helpers, CLI flags) is still flowing; 1.0 carries an API-stability promise we cannot keep yet.
- **Loose changelog (`git log --oneline` as the changelog).** Rejected: commit messages are oriented to authors/reviewers, not users. Keep a Changelog phrases changes from the user's perspective.

## Consequences

**Positive**
- A future user has a known-working version to pin.
- Breaking changes are concentrated at phase boundaries (per the milestone mapping), making upgrade decisions easier.
- `CHANGELOG.md` becomes an at-a-glance project history that complements the per-commit `git log`.

**Negative**
- Every behaviour-changing PR must touch `CHANGELOG.md` under `[Unreleased]`. We accept this as a documentation tax.
- Release cuts are manual until release tooling lands (future work).

**Locked in until superseded**
- Semver, the milestone mapping, and Keep a Changelog format are baseline. Switching to CalVer or different changelog format requires a new ADR.

## Future work

- Initial `CHANGELOG.md` generation summarizing v0.0.x history. Separate PR.
- Release script (`scripts/release.sh` or `cargo-release`) — automates tag, changelog rotation, and (eventually) artifact publish.
- Artifact distribution ADR (binary release vs `crates.io` vs runtime image) — when Phase 2 completes.
- Pre-release / RC tag conventions (e.g. `v0.1.0-rc.1`) — when first release approaches.

## References

- [Semantic Versioning](https://semver.org).
- [Keep a Changelog](https://keepachangelog.com).
- ADR 0006 (Phase 1 codegen) — defines what "Phase 1 complete" means.
- ADR 0124 (CI/CD) — CI gate that release commits must pass before tagging.
- `docs/PRD.jp.md` — phase definitions feeding the milestone mapping.
