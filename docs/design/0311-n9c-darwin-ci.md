# 0311. N9-C multi-target CI — arm64-darwin lane

- **Status:** Accepted (soft lane landed; bake + hard-fail flip pending CI)
- **Kind:** Architecture Decision
- **Date:** 2026-07-10
- **Deciders:** ShortArrow

## Context

ADR 0308 closed N9-A/B (native arm64-linux lane) and pinned N9-C: an
arm64-darwin lane. GitHub's `macos-15` runners are Apple Silicon and free
for public repos, so — as with N9-A — no cross-compilation is needed:
`llc` targets the host triple and every e2e test compiles, links (Apple
`cc` → Mach-O), and runs a native arm64-darwin binary. This is the first
lane exercising a non-ELF object format and libSystem instead of glibc.

## MLIR source: Homebrew bottle

macOS runners do not support job containers, so the Docker-image pattern
of the Linux lanes does not apply. Options:

- **Homebrew `llvm` bottle — chosen.** The formula builds with MLIR and
  Polly enabled and is melior's documented macOS path
  (`MLIR_SYS_*_PREFIX=$(brew --prefix llvm)`). Bottle install is minutes.
- MLIR source build in the job — ~1-2h per run with no image to cache
  into. Fallback only.

Risk accepted: Homebrew's unversioned `llvm` formula tracks latest, so it
will eventually drift past 22.x and break `MLIR_SYS_220_PREFIX`
discovery. The job therefore prefers `llvm@22` when that versioned
formula exists and **fails fast on a version guard**
(`llvm-config --version` must be 22.x) with an explicit error, rather
than letting mlir-sys produce a confusing link failure.

## Lane design

`clippy-and-test-darwin` in `ci.yml`, mirroring the arm64-linux lane:

- `runs-on: macos-15`; rustup via `dtolnay/rust-toolchain` (no image).
- Homebrew step resolves the formula, guards the version, exports
  `MLIR_SYS_220_PREFIX` / `LLVM_SYS_220_PREFIX` / `TABLEGEN_220_PREFIX`
  to the keg prefix, and prepends `$(brew --prefix)/bin` to `PATH` so
  `link.rs` finds `llc`.
- `RUST_MIN_STACK=8388608` — same AArch64 frame-spill headroom as the
  linux arm64 lane (ADR 0308 bake log).
- **`continue-on-error: true` during bake.** When the lane is green,
  the flip commit closes N9-C (same staged pattern as N9-B).

## Follow-up

1. First main push runs the lane; bake log records findings below.
2. When green: flip `continue-on-error` off — closes **N9-C**.
3. **N9-D** (release artifacts): tag-triggered multi-target binary job.
   Separate ADR.
