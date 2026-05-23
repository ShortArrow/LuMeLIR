# 0005. MLIR Integration Environment: WSL2 (Arch Linux) Primary, Windows Native Best-Effort

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

> **Status note (2026-05-10, ADR 0090):** Container-based reproducibility
> for the build environment remains **deferred** under this ADR's
> existing reject reasoning ("Dockerised Linux build on Windows host
> rejected. Adds Docker Desktop as a dependency for a box that
> already has WSL2"). ADR 0090 noted that the lighter-weight
> observability investment (`--emit` dump flags) lands first.
> Container e2e re-evaluation trigger: post-CI introduction or
> multi-contributor onboarding pressure. ADR 0005's decision is
> **not overturned** by this note.

## Context

Phase 1 PoC requires linking against MLIR / LLVM 22 via the `melior` crate. AGENTS.md §11 listed "Windows vs WSL2/Linux for MLIR builds" as TBD. We ran a Windows native spike (`V:/melior-spike/`) to find out how much work it would take.

Findings:

- LLVM / MLIR binary distributions for Windows do **not** ship a working MLIR toolchain. The official `LLVM.LLVM` winget package contains `llvm-c/` + `clang-c/` only — no `mlir/`.
- `conda-forge` ships `mlir=22.1.3` as a prebuilt Windows package (MSVC ABI). Installing it via Miniforge makes the MLIR C API and `llvm-config` reachable.
- Even with `conda-forge mlir` in place, the `melior` → `mlir-sys` → `tblgen` toolchain requires **7 separate patches and environment workarounds** before it even gets to final linking, and still fails at `z.lib` (zlib) resolution. See `V:/melior-spike/FINDINGS.md` for the full list.
- Each patch is reasonable and upstream-able, but fixing all of them (plus the ones that will appear after `z.lib`) is a multi-day effort with no direct payoff for the LuMeLIR PoC.
- WSL2 (Arch Linux) is already available on the development machine. The MLIR 22.1.3 AUR package exists (6 votes, Linux-native build). Standard distro build of MLIR on Linux is known to work with `melior` out of the box.

We must ship Phase 1 without turning into tblgen maintainers.

## Decision

**Phase 1 development happens in WSL2 Arch Linux.** Windows native MLIR integration is explicitly de-prioritised but kept as a long-term goal, tracked in `V:/melior-spike/` outside the LuMeLIR source tree.

### Concretely

- `V:/LuMeLIR/` is a Windows path but the Rust crate builds and runs identically under `/mnt/v/LuMeLIR` in WSL2 — no repo-level changes are required for the cross-environment setup.
- Primary `cargo build` / `cargo test` is invoked from **WSL2 Arch Linux** starting at Phase 1's MLIR integration step.
- The Windows tooling we already set up (Miniforge, `lumelir-mlir` conda env) is **left in place but unused** until someone resumes the Windows native push.
- `V:/melior-spike/` (not part of the LuMeLIR git repo) retains the 7 patches and `FINDINGS.md`. It is a scratchpad / upstream-PR nursery, not a LuMeLIR deliverable.
- Windows native support may return as a ADR-0010-ish "Windows CI + installer" story once (a) the remaining `z.lib` / DLL-on-PATH issues are solved and (b) the tblgen patches are accepted upstream or vendored cleanly.

### Required WSL2 packages (Arch)

- `base-devel` (gcc, make, binutils)
- `llvm 22.x` (from official `extra` repo) — note: Arch's `llvm` does **not** include MLIR
- `mlir` from AUR (`yay -S mlir` or `paru -S mlir`) — LLVM 22-compatible
- `rust` (or `rustup` with stable)
- `cmake`, `ninja`, `pkgconf`, `clang` — auxiliary build tools
- `zlib`, `zstd`, `libxml2` — linker dependencies for LLVM

Concrete install command:
```bash
sudo pacman -S --needed base-devel llvm rust cmake ninja pkgconf clang zlib zstd libxml2
paru -S mlir     # or yay -S mlir
```

### Env vars needed under WSL2

Drastically simpler than Windows:

```bash
export MLIR_SYS_220_PREFIX=/usr
export LLVM_SYS_220_PREFIX=/usr
export TABLEGEN_220_PREFIX=/usr
```

(Arch's AUR `mlir` installs to `/usr`.)

## Alternatives Considered

### Keep pushing Windows native through
Rejected for Phase 1. The spike enumerated 7 patches + 1 still-open failure; realistically several more link-stage issues remain (DLL-on-PATH, zstd/xml2 naming mismatches, mlir-sys own assumptions). None of this work advances the LuMeLIR PoC; it advances Windows-MSVC support for the Rust MLIR ecosystem as a whole. Valuable, but a separate project.

### Build LLVM+MLIR from source on Windows
Rejected. 40+ GB, 4-8 hours, plus the same MSVC/C++20/tblgen issues surface during consumption. No reason to pay the build cost and *also* carry all the tblgen patches.

### Dockerised Linux build on Windows host (no WSL2)
Rejected. Adds Docker Desktop as a dependency for a box that already has WSL2. WSL2 ↔ Windows filesystem sharing (`/mnt/v/`) is good enough; Docker's bind-mount performance and file-watching story is worse for iterative `cargo test`.

### macOS / pure Linux development environment
Out of scope for this ADR — the dev machine is Windows + WSL2 and that's not changing in Phase 1.

## Consequences

**Positive**
- Phase 1 can resume immediately with a battle-tested Linux MLIR build.
- No custom patches enter LuMeLIR's dependency tree; we consume `melior` unmodified.
- The Windows spike work is preserved for future upstream contribution and doesn't pollute LuMeLIR history.
- `cargo check` / `cargo test --lib` for non-codegen layers (`lexer`, `parser`, future `hir`, `mir`) still run on either environment — most of LuMeLIR is platform-independent Rust.

**Negative**
- Contributors without WSL2 (pure Windows or macOS) cannot build the `codegen` layer yet. Documented explicitly in AGENTS.md §10.3 and this ADR.
- File I/O from WSL2 into `/mnt/v/` is slower than a pure ext4 layout; accepted for now to keep a single source tree. If `cargo build` times become a problem, we revisit by moving to `~/LuMeLIR` inside WSL2.
- The setup depends on AUR's `mlir` package, which is community-maintained and could break at any LLVM major-version bump. Mitigation: Phase 1 pins to LLVM 22; we'll track AUR compatibility as part of `ADR XXXX: MLIR version pinning` when we update.

**Locked in until superseded**
- A future ADR (expected: 0009 or later, once Windows native link issues are fully resolved or the tblgen patches land upstream) will add Windows native as a second-class supported environment.
- If WSL2 becomes unavailable (major WSL2 regression, user switches to pure Linux), we re-run this decision with the new environment.

## References

- `V:/melior-spike/FINDINGS.md` — Windows native patch log (out-of-tree)
- AGENTS.md §10.3 — Environment gotchas (updated to mention WSL2)
- AGENTS.md §11 — TBD list (MLIR entry now links here)
- Arch AUR `mlir` package: <https://aur.archlinux.org/packages/mlir>
- `melior` crate: <https://github.com/mlir-rs/melior>
