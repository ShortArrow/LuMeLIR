# 0126. Security Policy

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

A compiler is a security-relevant artifact: it accepts attacker-controlled input (user Lua source, possibly from untrusted sources) and produces a native binary. Memory-safety bugs in the compiler can become arbitrary-code-execution vectors at compile time. LuMeLIR's Rust foundation provides default memory safety, but the MLIR/LLVM FFI requires `unsafe` and is the natural attack-surface concentration point.

There has been no explicit security policy until now. Practice (confining `unsafe` to MLIR FFI, never committing secrets) has been ad-hoc.

## Decision

### `unsafe` discipline

- `unsafe` blocks are confined to MLIR/LLVM FFI boundaries (`src/codegen/`).
- Every `unsafe` block carries a `// SAFETY: <reason>` comment immediately above. The comment must explain *why* the unsafety is sound — what invariant the caller upholds.
- `forbid(unsafe_code)` is **not** applied crate-wide (MLIR FFI requires it), but `unsafe` outside `codegen` requires an ADR justification.

### `unwrap` / `expect` discipline

- `unwrap()` / `expect()` are forbidden in non-test code unless justified by a comment explaining why the invariant cannot be violated.
- Library-layer errors propagate via `Result` (`thiserror` enums per ADR 0003); the CLI layer is the only place panic-on-failure (`anyhow` collapse) is acceptable.

### Dependency review

- Adding a crate dependency requires (per ADR 0128):
  - License check (Apache-2.0 / MIT / BSD acceptable; copyleft requires explicit ADR justification).
  - Maintenance status check (last release within ~18 months, or explicit "this is a stable abandonment" note).
  - `cargo tree` review for transitive dependencies.
- `cargo audit` runs in CI (follow-up PR after ADR 0124 workflow lands).

### Secrets

- No secrets in the repository. Environment variables, `.env` files (gitignored), or external secret managers only.
- `.gitignore` covers `.env`, `target/`, `*.log`, and IDE-local config.
- `git-secrets` pre-commit hook is recommended (not enforced for contributors who use signing/verification differently); the policy is the rule, the tooling is convenience.

### Vulnerability response

- Security issues are reported via GitHub Security Advisories (private channel), not public issues.
- Acknowledgement target: 7 days. Patch / mitigation target depends on severity (no formal SLA pre-1.0).
- Coordinated disclosure: standard 90-day window; can be shortened for actively-exploited issues.

## Alternatives considered

- **`forbid(unsafe_code)` crate-wide with `mlir` as a separate crate.** Rejected (for now). The crate split adds workspace ceremony for a project that already isolates `unsafe` to `codegen`; revisit if the unsafe surface grows.
- **No policy** (rely on Rust's default safety + good judgment). Rejected. Memory safety is a property the codebase must demonstrate, not assume — explicit `SAFETY:` comments are the audit trail.
- **Public issues for vulnerability reports.** Rejected. Pre-public-fix disclosure window is essential for coordinated remediation.
- **Strict SLA pre-1.0.** Rejected as premature. Solo / small-team project cannot reasonably commit to a fixed SLA before infrastructure exists; "best effort, ack within 7 days" is honest.

## Consequences

**Positive**
- Every `unsafe` block is auditable — grep for `// SAFETY:` produces a complete index.
- Dependency creep is hard (ADR 0128 + cargo audit + manual review).
- The secret-exposure failure mode is closed by `.gitignore` + (recommended) `git-secrets`.
- Reporters have a clear channel for vulnerabilities.

**Negative**
- The `// SAFETY:` comment requirement adds friction to legitimate MLIR FFI changes. Accepted: this is exactly the friction we want.
- `cargo audit` will occasionally flag transitive vulnerabilities we cannot directly fix. The mitigation policy is documented in the audit CI workflow (follow-up).

**Locked in until superseded**
- `unsafe` confinement, `SAFETY:` comments, secret exclusion are baseline.
- SLA and `cargo audit` adoption are baseline but their specifics may evolve in successor ADRs.

## Future work

- `.github/SECURITY.md` — public security policy file in repo (separate PR; this ADR is the source).
- `cargo audit` CI integration — workflow file in follow-up PR.
- `git-secrets` hook installation script — `scripts/install-hooks.sh` (separate PR).
- Formal SLA when the project reaches 1.0 / production users.

## References

- `CONTRIBUTING.md` §3.4 "Rust-Specific Guidance" — `unsafe` + `unwrap` rules (current rules).
- ADR 0003 (error handling) — `thiserror` / `anyhow` boundary.
- ADR 0128 (dependency policy) — license/maintenance review.
- ADR 0124 (CI/CD) — where `cargo audit` runs.
- [rustsec.org](https://rustsec.org) — Rust security advisory database.
