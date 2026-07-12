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

## Bake log (2026-07-11)

1. Homebrew step green on round 1: formula `llvm` is 22.1.8, version
   guard passed, melior/mlir-sys compile against the bottle. Lane failed
   on a **toolchain skew**, not darwin: this lane tracks latest stable
   (Rust 1.97) while the Linux lanes pin via baked images, so the new
   `useless_borrows_in_formatting` lint fired once (`4d79b37`).
2. Round 2 reached `cargo test` and died at link: `ld: write() failed,
   errno=28` (ENOSPC). mlir-sys' auto-detect picks **static** libs, so
   all ~150 integration-test binaries carried full LLVM+MLIR and
   exhausted the runner disk. Fix: `MLIR_SYS_LINK_SHARED=1` — the bottle
   ships `libMLIR.dylib`, and shared mode links `-lMLIR -lMLIR-C`
   instead. Also `MACOSX_DEPLOYMENT_TARGET=15.0` to stop per-object
   "built for newer macOS" linker warnings (rustc defaults to 11.0).

3. Round 3: shared mode fails with `ld: library 'MLIR-C' not found` —
   the bottle ships `libMLIR.dylib` but **no `libMLIR-C.dylib`**, and
   mlir-sys' shared mode unconditionally links both. Probe data:
   `llvm-config --shared-mode` = shared, runner disk 43Gi free (so
   static's ~150 full-toolchain binaries genuinely cannot fit). Fix:
   the brew step verifies the C API symbols live inside
   `libMLIR.dylib` (`nm -gU … | grep mlirContextCreate`) and shims
   `libMLIR-C.dylib -> libMLIR.dylib`; both `-l` flags then resolve to
   the same install name and ld dedups. Fails fast with an explicit
   error if the C API ever moves out.
4. Round 4: the symlink guard tripped — `libMLIR.dylib` does **not**
   export the C API; the bottle ships it only as static
   `libMLIRCAPI*.a` archives (Homebrew builds without
   `MLIR_BUILD_MLIR_C_DYLIB`). Fix: the brew step now builds
   `libMLIR-C.dylib` itself — `clang++ -dynamiclib` force-loading every
   `libMLIRCAPI*.a` against `-lMLIR` + shared LLVM, i.e. the same
   artifact upstream's `MLIR_BUILD_MLIR_C_DYLIB=ON` produces — then
   verifies `mlirContextCreate` is exported.
5. Round 5: shim link failed on `libMLIRCAPIExecutionEngine.a` —
   `mlir::ExecutionEngine::*` is not in `libMLIR.dylib` (upstream also
   excludes the JIT-side ExecutionEngine from the aggregate dylib).
   LuMeLIR is AOT (`llc` + `cc`, never `mlirExecutionEngine*`), so the
   shim simply skips that one archive; unreferenced extern
   declarations in the mlir-sys bindings cost nothing at link time.
6. Round 6: toolchain fully green (brew step, clippy, full suite
   runs); every e2e test fails at the generated binary's `cc` link:
   `Undefined symbols: "_stdout"` — libSystem names the stdio globals
   `__stdoutp`/`__stdinp` (glibc exports the POSIX names). First
   genuine codegen finding of the lane; fixed in **ADR 0312**.

## Follow-up

1. First main push runs the lane; bake log records findings above.
2. When green: flip `continue-on-error` off — closes **N9-C**.
3. **N9-D** (release artifacts): tag-triggered multi-target binary job.
   Separate ADR.
