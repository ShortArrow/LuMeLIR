# 0128. Dependency Addition Policy

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

Each crate dependency is a footprint: a license to track, a security surface, a maintenance status to watch, transitive dependencies to audit, and a compile-time cost to bear. LuMeLIR's existing dependencies (clap, melior, llvm-sys, mlir-sys, tablegen-sys, thiserror, anyhow) all landed with corresponding ADRs (or were Phase 0 baseline). The policy that drove those additions has not been recorded — this ADR codifies it.

## Decision

### When to add a dependency

- **Add a dependency at the moment it is first needed**, never speculatively.
- The PR that adds the dependency to `Cargo.toml` must also contain the first use site. No "we might use this for X later" additions.
- Adding a dependency requires (or is the subject of) an ADR. For small, obvious cases (e.g. a tiny utility crate replacing 10 lines of bespoke code) a brief mention in the feature ADR is sufficient; for foundational dependencies (parser library, codegen framework) a dedicated ADR is required.

### What the ADR must cover

1. **What problem the dependency solves** that we cannot reasonably solve in-tree.
2. **Alternatives considered**, including the "build it ourselves" option, with rejection reasons.
3. **Licensing**: Apache-2.0 / MIT / BSD-style permissive is acceptable. Copyleft (GPL, AGPL) requires explicit justification.
4. **Maintenance status**: last release date (within ~18 months) or explicit "this is a stable abandonment" rationale (e.g. a small frozen utility crate).
5. **Transitive footprint**: output of `cargo tree -p <new-dep>` summarized — count of new transitives, any concerning ones.
6. **Version policy**: pinned major + minor (`^0.27` style) vs strict pin; rationale.

### Cargo hygiene

- `cargo tree` review is mandatory before the PR merges.
- `Cargo.lock` is committed (binary crate). Do not hand-edit; let `cargo` regenerate via `cargo update -p <crate>` for intentional bumps.
- `cargo audit` runs in CI (ADR 0124 + 0126 follow-up) to surface advisories.
- Removing a dependency is also an ADR-worthy decision if it changes a public API surface; otherwise it can ride in the same PR as the code change that removes its last use.

## Alternatives considered

- **"Future-proof" speculative additions** (add deps you "might want"). Rejected. YAGNI; abandoned dependency stubs are a maintenance liability and a security surface for no benefit.
- **No policy** (add deps freely). Rejected. Compiler frontends are dependency-light by design; an unpoliced policy leads to bloat that hurts compile time and review surface.
- **Vendoring all deps in-tree** (no `[dependencies]`). Rejected as extreme; loses upstream patch flow for security fixes.
- **Workspace dependencies only via a top-level `[workspace.dependencies]` table** (centralized version control). Considered for future workspace split (per ADR 0121 future work), not applicable yet.

## Consequences

**Positive**
- Each line of `Cargo.toml` is justified by a discoverable ADR.
- `cargo tree` stays scannable; transitive bloat is caught early.
- Security audits (ADR 0126) have a bounded surface to track.

**Negative**
- A small dependency that "would have been useful" can feel blocked by ADR ceremony. Mitigation: brief mention in the feature ADR suffices for small cases.
- Dependency bumps (security fixes) move fast through CI but require the same `cargo tree` review for transitive changes. Accept as part of the maintenance discipline.

**Locked in until superseded**
- The "add-when-needed + ADR mention" rule is baseline.
- Audit cadence (CI per-PR) is baseline once ADR 0124 + 0126 follow-up lands.

## References

- `CONTRIBUTING.md` "Dependency Addition Policy" — current rules.
- ADR 0003 (error handling) — example of dependency-justifying ADR (`thiserror`, `anyhow`).
- ADR 0005 (MLIR environment) — example of major dependency family (`melior`, `mlir-sys`, `llvm-sys`, `tablegen-sys`).
- ADR 0124 (CI/CD) — where dependency review runs.
- ADR 0126 (security policy) — license / advisory considerations.
