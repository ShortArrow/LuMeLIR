# Security Policy

LuMeLIR's full security policy is recorded as [ADR 0126](../docs/design/0126-security-policy.md). This file is the GitHub-visible summary.

## Reporting a vulnerability

Use [GitHub Security Advisories](../../security/advisories/new) for private disclosure. **Do not file public issues for security bugs.**

Expected response:

- Acknowledgement within 7 days.
- Coordinated disclosure: standard 90-day window; shortened for actively exploited issues.

## Supported versions

Pre-1.0 (`v0.0.x` — current). All security fixes target `main`. There is no LTS branch yet; once `v1.0` ships a support-window policy will be recorded as a successor ADR to [ADR 0125](../docs/design/0125-release-procedure.md).

## Scope

The compiler (`lumelir`) and its public Rust crate surface are in scope. Generated artifacts (the native binaries this compiler emits) are out of scope — vulnerabilities in user-provided Lua sources or in linked C libraries should be reported to those projects.
