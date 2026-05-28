# 0132. CI Image Base Distro: Arch + AUR `mlir`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-28
- **Deciders:** ShortArrow

## Context

[ADR 0124](0124-ci-cd-policy.md) committed to GitHub Actions as the CI provider with `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` as the gate, but explicitly deferred the *how* of getting an MLIR-22 environment onto the runner:

> MLIR-linked test execution requires an environment with MLIR 22 (matching melior 0.27, per ADR 0005). Two acceptable approaches:
>   - Self-hosted runner on a WSL2 Arch host with the toolchain pre-installed (low setup cost for the project lead; not portable to forks).
>   - Docker image with MLIR 22 prebuilt (portable; future PR will publish it under `ghcr.io/<owner>/lumelir-ci`).
>   The decision between the two is deferred to the workflow-implementation PR; both satisfy this policy.

The first implementation attempt (commit `5570500`) installed MLIR via `apt.llvm.org`. CI runs `26340196055` (Polly missing), `26350885090` (libpolly added, then dialect `.td` includes missing), confirmed that `libmlir-22-dev` does **not** ship the dialect TableGen source files (`mlir/Dialect/Transform/IR/TransformOps.td`, `Vector/IR/VectorOps.td`, `X86Vector/X86Vector.td`, and 29 others) that `melior_macro::dialect` needs at compile time. apt.llvm.org's MLIR packaging is binary-runtime-oriented; the development sources it ships are incomplete for downstream `mlir-tblgen` consumers.

`continue-on-error: true` had been masking this as "tolerated red" while the install path was iterated. A real CI gate requires either fixing the source-availability problem or switching distros.

## Decision

CI's `clippy-and-test` job runs inside a Docker image at `ghcr.io/shortarrow/lumelir-ci:mlir-22`. The image is built from `.github/docker/Dockerfile.ci` by `.github/workflows/build-ci-image.yml`, triggered only when the Dockerfile or that workflow itself changes.

Image composition:

- Base: `archlinux:base-devel` — matches the WSL2 Arch dev environment from [ADR 0005](0005-mlir-environment.md) one-for-one.
- LLVM toolchain via pacman: `llvm cmake ninja pkgconf clang zlib zstd libxml2` (same list as `CONTRIBUTING.md` "Setup").
- MLIR via AUR: `git clone https://aur.archlinux.org/mlir.git && makepkg -si`. PGP signature on the upstream LLVM tarball is verified against key `316C56D064CACBA5` imported from `keyserver.ubuntu.com` (per [ADR 0126](0126-security-policy.md) supply-chain posture).
- Rust toolchain via `rustup default stable` + clippy + rustfmt, installed system-wide at `RUSTUP_HOME=/usr/local/rustup` and `CARGO_HOME=/usr/local/cargo` so it works regardless of which user GitHub Actions runs the container as (it forces root).
- Env: `MLIR_SYS_220_PREFIX=LLVM_SYS_220_PREFIX=TABLEGEN_220_PREFIX=/usr`, mirroring `CONTRIBUTING.md`.

The CI workflow pulls the image via `container:` + `credentials:` with `${{ secrets.GITHUB_TOKEN }}`, which works whether the ghcr.io package is private or public.

## Alternatives considered

### apt.llvm.org / Ubuntu

Rejected. Empirically incomplete: CI run `26350885090` proved `libmlir-22-dev` is missing the dialect `.td` source files that `melior_macro::dialect` consumes during `cargo build`. Switching from `libmlir-22-dev` to building those files into a sidecar layer would re-implement what AUR's `mlir` PKGBUILD already does cleanly. The apt path also requires a fresh `sudo apt-get update + install` on every CI run, costing minutes per run.

### MLIR from-source via `cmake` inside CI

Rejected. A from-source LLVM+MLIR build is ~1-2 hours on a 4-core ubuntu-latest runner. Done inline in every `cargo test` job, that's prohibitive. Caching the build output across CI runs would re-create the same image artifact we're producing here, just less cleanly (loose tarballs vs. an OCI image with proper layering).

### Self-hosted Arch runner

Rejected. ADR 0124 already weighed this:

> Self-hosted runner on a WSL2 Arch host with the toolchain pre-installed (low setup cost for the project lead; not portable to forks).

The portability cost is decisive: a fork or future contributor can't replicate the runner, and the lead's availability becomes a CI dependency.

### Ubuntu + manually populated MLIR source tree

Rejected as too ad-hoc. Building a "frankenstein" MLIR install by combining apt binaries with hand-copied source files (or a wholesale `tar xf llvm-project-22.tar.xz`) duplicates work the AUR PKGBUILD already does correctly. Brittle to upstream layout changes.

## Consequences

**Positive**

- CI gate (`fmt + clippy + test`) becomes a real required gate, not a tolerated-red signal.
- Image composition mirrors the WSL2 Arch dev env exactly, so "works locally" and "works in CI" mean the same thing.
- AUR-driven MLIR build picks up upstream LLVM patches when the AUR maintainer updates the PKGBUILD; we get refreshes for free without managing a fork.
- PGP verification of the LLVM source tarball aligns with ADR 0126 supply-chain stance.

**Negative**

- First image build takes ~2 hours (MLIR from source). Subsequent builds with cached layers complete in seconds when only later layers change (e.g. Rust toolchain tweak — empirically 1m38s in run `26473501594`).
- Every Dockerfile change pays the rebuild cost. Mitigation: changes are rare.
- The runner's `cargo` cache is fresh inside the container on each run. `Swatinem/rust-cache@v2` caches the registry and `target/` across runs.
- Image is built only on linux/amd64. Arm support (e.g. Apple Silicon contributors using act/locally) is future work.

**Locked in until superseded**

- Image registry is `ghcr.io/shortarrow/lumelir-ci`. Tags are `mlir-22` (canonical) and `latest`.
- AUR `mlir` is the canonical MLIR source. Pinning to a specific package version is future work if drift becomes a problem.

## Future work

- `cargo audit` CI step (warn-not-fail, then required) — `cargo audit` doesn't require MLIR, can run in the fmt job or a separate job. [ADR 0126](0126-security-policy.md) future work.
- Multi-arch image (`linux/arm64`) if a contributor needs it.
- BuildKit registry-side cache (`cache-to: type=registry,ref=...`) for incremental rebuilds across actors.
- Dependabot for `actions/checkout`, `docker/build-push-action`, etc. Workflow file dependency hygiene.
- Branch protection on `main` requiring CI green (now that the gate is real).
- Pinned MLIR version in Dockerfile (e.g. `mlir=22.x.y-r1`) if AUR drift breaks compatibility with melior 0.27.

## References

- [ADR 0005](0005-mlir-environment.md) — established WSL2 Arch as the dev environment baseline.
- [ADR 0124](0124-ci-cd-policy.md) — committed to GitHub Actions, deferred the runner-vs-image choice this ADR closes.
- [ADR 0126](0126-security-policy.md) — supply-chain stance that motivates the PGP key import.
- CI run `26350885090` — evidence that `apt.llvm.org`'s `libmlir-22-dev` ships incomplete dialect sources.
- CI run `26357456773` — first successful image build (2h8m37s, full MLIR from source).
- CI run `26473501594` — cached rebuild (1m38s, Rust layer only).
- `.github/docker/Dockerfile.ci` — image source of truth.
- `.github/workflows/build-ci-image.yml` — image build + push workflow.
- `.github/workflows/ci.yml` — consumer of the image via `container:`.
