# 0313. N9-D release artifacts — tag-triggered multi-target binaries

- **Status:** Accepted (bake green 2026-07-12 — N9-D closed, N9 arc complete)
- **Kind:** Architecture Decision
- **Date:** 2026-07-12
- **Deciders:** ShortArrow

## Context

N9-A/B/C/E (ADR 0308/0310/0311/0312) established three green CI lanes:
x86_64-linux, aarch64-linux, arm64-darwin. N9-D closes the N9 arc by
publishing `lumelir` binaries per target when a version tag is pushed.

## Decisions

- **Trigger**: tag push `v*` publishes a release. `workflow_dispatch`
  runs the identical pipeline but publishes a **draft** release —
  that is the bake path (verify, then delete the draft) and keeps the
  real tag path proven-by-construction.
- **Build environments mirror the CI lanes** (same images / brew
  step), so a release build can only fail in packaging, never in a
  configuration CI hasn't already proven. Linking stays whatever each
  lane's mlir-sys auto-detect (or lane env) chose: Arch/amd64 links
  the system `libMLIR-C.so`, apt/arm64 links static, darwin builds
  one static binary (the ADR 0311 ENOSPC problem was ~150 test
  binaries; a single release binary is fine and avoids `/opt/homebrew`
  path dependencies).
- **Artifacts**: `lumelir-<tag>-<triple>.tar.gz` (binary + LICENSE-* +
  README) for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `aarch64-apple-darwin`, plus a `SHA256SUMS` file. Published with the
  `gh` CLI (preinstalled; no third-party release action, per the
  ADR 0126 supply-chain posture).
- **Runtime requirements are documented, not eliminated**: `lumelir`
  shells out to `llc` and `cc` (ADR 0117 pipeline), and the linux
  builds additionally need the distro's LLVM/MLIR 22 shared libs. The
  release body states this. Fully static distribution is out of scope
  for N9-D.

## Bake log (2026-07-12)

Dispatch run `29191133255` green on the first attempt: all three
builds + publish succeeded; draft `v0.0.0-bake` carried the three
tarballs + `SHA256SUMS`. Local verification of the amd64 artifact:
checksum matches, tarball layout and executable bit correct; running
it requires `libMLIR-C.so.22` exactly as the release notes state
(shared-linked lane). Sizes: amd64 2.4 MB (shared MLIR), arm64-linux
43 MB / darwin 54 MB (static MLIR). Draft deleted after verification.

## Follow-up

1. First real `v*` tag exercises the publish path end-to-end.
