# Phase 1 Handover — Switching to WSL2 (Arch Linux) for MLIR Integration

**Audience:** the next Claude Code (or any LLM agent) session that resumes LuMeLIR work, started from inside WSL2 Arch rather than the Windows host.

**Status at handover:** lexer + parser complete under Windows native (19 unit tests green), all committed. Phase 1 now enters the MLIR codegen stage, for which the primary dev environment moves to WSL2 per [ADR 0005](../design/0005-mlir-environment.md).

## 1. Where you are

- `/mnt/v/LuMeLIR/` — the LuMeLIR git repo (same working tree as Windows `V:/LuMeLIR/`).
- `/mnt/v/melior-spike/` — out-of-tree Windows spike. **Do not touch from WSL2**; it records Windows-side patches for later upstream PRs. See `/mnt/v/melior-spike/FINDINGS.md`.
- Git on `/mnt/v/` works correctly (shared `.git` dir). Expect slightly slower file I/O than native ext4.

## 2. What is already done

Committed to `main` (as of this handover):

| SHA prefix | Topic |
|---|---|
| `30956dd` | Initial scaffold (Rust 2024 edition, CLI skeleton) |
| `937ab9d` | Track `.claude/.gitignore` |
| `cfb3051` | `AGENTS.md` / `CLAUDE.md` / `CONTRIBUTING.md` |
| `913ce20` | ADRs 0001-0003 (lexer hand-written, `lib.rs` layering, error handling) |
| `80a817d` | Hand-written lexer (`src/lexer/*`, 8 tests) |
| `77b375e` | ADR 0004 (parser implementation) |
| `e7e5e47` | Hand-written parser (`src/parser/*`, 11 tests) |

Run `git log --oneline` to confirm. All 19 tests green under Windows; they will stay green under WSL2 (pure Rust, no FFI yet).

## 3. Bootstrap the WSL2 environment

### 3.1 Packages

```bash
# Official repo — no MLIR here; Arch's `llvm` package ships LLVM only.
sudo pacman -Syu --needed
sudo pacman -S --needed base-devel llvm rust cmake ninja pkgconf clang \
                        zlib zstd libxml2 git openssh

# AUR helper (if you don't have one yet). Either paru or yay works.
sudo pacman -S --needed fakeroot debugedit
#   If no paru/yay installed:
#     git clone https://aur.archlinux.org/paru.git /tmp/paru && cd /tmp/paru && makepkg -si

# MLIR 22 from AUR (matches melior 0.27).
paru -S mlir       # or: yay -S mlir
```

AUR `mlir` installs to `/usr` (headers under `/usr/include/mlir*`, libs under `/usr/lib/libMLIR*`, tools like `mlir-tblgen` on PATH).

### 3.2 Rust toolchain

If `rustc --version` reports < 1.85, install via rustup instead of pacman:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# then: rustup default stable
```

Minimum required: Rust 1.85 (for edition 2024). We don't pin a version; whatever stable ships.

### 3.3 Env vars for melior

Put in `~/.bashrc` (or `~/.zshrc`) **or** a repo-local script if you prefer not to pollute the shell profile:

```bash
export MLIR_SYS_220_PREFIX=/usr
export LLVM_SYS_220_PREFIX=/usr
export TABLEGEN_220_PREFIX=/usr
```

These tell `mlir-sys`, `llvm-sys`, and `tblgen` where to find `llvm-config` and headers.

### 3.4 Sanity check

```bash
cd /mnt/v/LuMeLIR
cargo fmt --check                              # clean
cargo clippy --all-targets -- -D warnings      # clean
cargo test --lib                                # 19 tests, all green
cargo run -- --help                             # clap help output
llvm-config --version                           # 22.x
mlir-tblgen --version                           # 22.x
```

If all of those pass you're ready.

## 4. Next task (Phase 1 MLIR codegen — first slice)

**Goal:** compile `print(1 + 2)` all the way to a native binary through MLIR.

### 4.1 What's not decided yet

The first real codegen work requires an ADR (expected number: **0006**). Open questions:

- Dialect strategy: emit directly into `arith` + `func` + `llvm` dialects, or define a thin `lumelir` dialect first? For the Phase 1 PoC, direct use of standard dialects is likely enough. A custom dialect is a Phase 2 concern.
- Where does `codegen` live? Plan from ADR 0002 says `src/codegen/` inside the library. Keep that.
- How does `print` get linked? Options: (a) call the host `printf`, (b) embed a tiny libc wrapper, (c) invoke via MLIR execution engine / JIT. For a native binary we probably want `llc` → `.o` → system `cc` driver. Decide in ADR 0006.
- Runtime: do we link `-lc` or go fully freestanding? Phase 1 accepts `-lc`; freestanding is a Phase 2+ question tied to the MCU target.

### 4.2 Suggested first sub-task (red-green-refactor)

Under WSL2:

1. **Spike**: stand up a minimal `melior` crate in a scratch dir (`~/melior-hello/`) and emit an MLIR module that `mlir-translate -mlir-to-llvmir` turns into a valid LLVM IR file. No LuMeLIR code yet — just prove the toolchain works.
2. **Write ADR 0006** based on what the spike revealed: dialect choice, `codegen` module shape, how MLIR → LLVM IR → native link is wired.
3. **Add `codegen` module to LuMeLIR**: `src/codegen/mod.rs` behind `pub mod codegen;` in `src/lib.rs`. Add the `melior` dependency via `cargo add melior` in the same PR as ADR 0006.
4. **Red**: integration test `tests/phase1_print.rs` that calls `lumelir compile examples/hello.lua -o /tmp/hello && /tmp/hello` and expects stdout `3`. It will fail until codegen is wired.
5. **Green**: hook parser → codegen → `mlir-translate` → system `cc`. Keep it ugly; make the test pass.
6. **Refactor**: tidy spans, error types, clean up the `cc` invocation into something that makes sense on Linux first (cross-compile is a later concern).

This is a multi-session scope. The first session from WSL2 should target step 1 + step 2 (ADR 0006 draft).

## 5. Work to preserve from the Windows attempt

`/mnt/v/melior-spike/FINDINGS.md` enumerates **7 concrete patches** that were needed to coax Melior onto Windows MSVC. The work stopped at `z.lib` (zlib name resolution), with several more link-stage issues likely remaining. Do not re-do this; it's not LuMeLIR's job to finish it. If you (or a future contributor) want to chip away at Windows native support:

1. Fork `mlir-rs/tblgen-rs` on GitHub.
2. Apply the 5 `build.rs` patches from `FINDINGS.md`.
3. Resolve `z.lib` (likely: add `conda-forge zlib` and/or rewrite `z` → `zlib` in the system-libs loop).
4. PR upstream, and only then consider bringing Windows native back into LuMeLIR's supported envs.

This is tracked under AGENTS.md §11 TBD "Windows native MLIR support".

## 6. Invariants that did not change

- Coding principles from AGENTS.md §4 (FP, CA, TDD) apply identically in WSL2.
- Conventional Commits still enforced.
- No commits without the user's explicit instruction (§10.4).
- PRD.jp.md is still SoT.
- Do not edit `/mnt/v/LuMeLIR/.claude/settings.local.json` or `git config`.

## 7. Known annoyances under WSL2

- `/mnt/v/` reports file changes to inotify with delay; if an editor watcher (`cargo watch`) seems slow, expect it.
- Windows Defender may scan `target/` and slow things. If build times hurt, consider `cargo install cargo-target-dir-redirect`-style tricks or a pure-WSL ext4 clone.
- The Windows-side LIBCLANG_PATH env var leaking into WSL2 shells: usually harmless (WSL2 has its own env), but if `bindgen` ever complains about a weird libclang, `unset LIBCLANG_PATH` first.

## 8. What to do at session start (quick checklist)

```bash
cd /mnt/v/LuMeLIR
cat docs/handover/phase1-wsl2-migration.md        # this file
cat AGENTS.md                                      # current conventions
cat docs/design/0005-mlir-environment.md           # why we're here
git log --oneline | head -10                       # recent history
cargo test --lib                                   # confirm green
```

Then tackle §4.2 step 1 (the melior-hello spike in a scratch dir) and report back.

## 9. If you hit something not covered here

Ask the user. Per AGENTS.md §10.5: a short question beats a long wrong implementation. In particular, do not invent answers for ADR 0006 — we need a real decision, not a guess.
