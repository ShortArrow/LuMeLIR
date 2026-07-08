# 0309. cargo-audit policy exceptions

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-09
- **Deciders:** ShortArrow

## Context

The ADR 0251 audit gate went red on two advisories unrelated to project code:

1. **RUSTSEC-2026-0190** — `anyhow` unsoundness in `Error::downcast_mut()`. Fixed upstream; resolved by `cargo update -p anyhow` (1.0.102 → 1.0.103).
2. **RUSTSEC-2024-0436** — `paste` is unmaintained (informational, no vulnerability). Transitive via melior 0.27 → melior-macro → tblgen 0.9.1; no upstream fix exists to update into.

## Decision

- Dependency updates are always preferred when a fixed version exists (anyhow case).
- For advisories with **no actionable fix** and **no vulnerability** (unmaintained/informational class), add an explicit ignore to `.cargo/audit.toml` with an inline justification comment naming the transitive path and the re-evaluation trigger (here: each melior upgrade).
- Never ignore an advisory with an actual CVSS/vulnerability classification — those block until a fix or a dependency swap.

## References

- ADR 0251 — the audit gate.
- RUSTSEC-2024-0436 / RUSTSEC-2026-0190.
