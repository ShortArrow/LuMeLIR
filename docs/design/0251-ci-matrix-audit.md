# 0251. CI Matrix + Security Audit Step

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M16 sub-ADR. Phase 4 closed (ADR 0250); Phase 5 (ADR 0195 §Workstreams) opens its production-hardening items. The four sub-items are:

- CI matrix (multi-target QA).
- Release artifacts (signed binaries, changelog automation).
- Security review (allocator + bridge + libc usage).
- `unsafe`-block audit — already shipped in ADR 0249.

This ADR pins the CI matrix decision + adds a concrete security-advisory check via `cargo audit`. Release artifacts ship in [ADR 0252](0252-phase5-close-declaration.md) alongside the Phase 5 close declaration.

## Scope (literal)

- ✅ Add a `cargo audit` job to `.github/workflows/ci.yml`. Runs on every PR + push; fails on any RustSec advisory at warn-level or above.
- ✅ Document the multi-target CI matrix plan: x86_64-linux (active), aarch64-linux (queued), arm64-darwin (queued), x86_64-windows (queued).
- ✅ Pin x86_64-linux as the active gate. The other three targets are queued — they require per-target MLIR 22 toolchain availability + a cross-build story, both of which are non-trivial setup work.
- ❌ Activate the aarch64-linux / macOS / Windows runners now. Each needs the equivalent of `.github/docker/Dockerfile.ci`'s MLIR 22 setup; deferred until the per-target toolchain investment is justified by a real demand for that target.
- ❌ Per-target benchmark gating. The existing CI runs the benchmark harness (ADR 0194) for correctness only; performance regression-detection (ADR 0193 §Workstream) needs a baseline-storage decision that is out of scope.
- ❌ Codesigning / notarisation. Future ADR if and when macOS release artifacts ship.

## Decision

### Active CI gate

```yaml
jobs:
  fmt: cargo fmt --check
  clippy-and-test:
    container: ghcr.io/shortarrow/lumelir-ci:mlir-22  # MLIR 22 baked in
    - cargo clippy --all-targets -- -D warnings
    - cargo test --no-fail-fast
  audit:                                              # NEW
    - cargo install cargo-audit --locked
    - cargo audit --deny warnings
```

The audit job runs on the host runner (no container needed — it only reads `Cargo.lock`). `--deny warnings` upgrades RustSec warn-level advisories (unmaintained / yanked) into CI failures, so any introduced dependency that the RustSec DB flags blocks merge until addressed.

### Multi-target matrix plan

| Target | Status | Blockers / decisions |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ✅ active | Existing CI image (ADR 0132). |
| `aarch64-unknown-linux-gnu` | ⏸ queued | Needs aarch64 GHA runner OR cross-compile + emulated test. MLIR 22 prebuilt for aarch64 from AUR or self-build. |
| `aarch64-apple-darwin` (M1+ macOS) | ⏸ queued | Needs macOS GHA runner with brew-installed MLIR 22. |
| `x86_64-pc-windows-msvc` | ⏸ queued | Windows MLIR build is the largest cost — defer until concrete user demand. |

Activation order when triggered: aarch64-linux → arm64-darwin → x86_64-windows. The same `cargo clippy + test + audit` gate applies to each new target.

### Why `--deny warnings`

RustSec warn-level advisories cover unmaintained crates and yanked versions. They don't always indicate exploitable vulnerabilities but represent a real supply-chain risk. Gating on them keeps `Cargo.lock` clean at the cost of occasional "upgrade or skip" friction; the alternative (only failing on critical / high advisories) leaves the door open for slow-burn issues that surface as breaking changes during a release prep.

## Tests

No new application tests. The CI gate change is metadata + workflow YAML.

The audit job runs against the existing `Cargo.lock` and validates that no current dependency carries an open advisory. If it fails on the first run after merge, that's a real signal to update dependencies — not a regression of this ADR.

## Test count delta

```
Step 0:  1617 (after ADR 0250)
C1 (docs + workflow YAML): 1617 → 1617
```

## References

- [ADR 0132](0132-ci-image-baked-mlir.md) — current CI image with MLIR 22 baked in.
- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry referencing production hardening as Phase 5 scope.
- [ADR 0195](0195-phase5-entry-criteria.md) — Phase 5 entry; CI matrix workstream this ADR closes.
- [ADR 0249](0249-unsafe-block-audit.md) — sibling Phase 5 hardening item (already shipped).
- [RustSec Advisory DB](https://rustsec.org/) — source for `cargo audit` advisories.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M16 milestone.
