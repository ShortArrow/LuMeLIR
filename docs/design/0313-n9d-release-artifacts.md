# 0313. N9-D release artifacts — tag-triggered multi-target binaries

- **Status:** Accepted (workflow landed; dispatch bake pending)
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

## Follow-up

1. `workflow_dispatch` bake: draft release created, artifacts
   verified, draft deleted. Findings go in the bake log below.
2. First real `v*` tag exercises the publish path end-to-end.
