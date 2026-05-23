# Contributing to LuMeLIR

LuMeLIR is a Rust-based compiler toolchain that lowers Lua through MLIR into native AOT binaries. Product vision: [`docs/PRD.jp.md`](docs/PRD.jp.md) (Japanese SoT) or [`docs/PRD.md`](docs/PRD.md) (English translation).

This file is the **canonical working conventions** for the repository. Both human contributors and LLM coding agents follow it. LLM-specific safety guardrails layered on top live in [`AGENTS.md`](AGENTS.md).

## Before You Start

1. [`docs/PRD.jp.md`](docs/PRD.jp.md) — product intent (SoT)
2. [`docs/design/README.md`](docs/design/README.md) — ADR conventions and chronological index
3. `docs/design/NNNN-*.md` — any ADRs relevant to your task
4. This file
5. Existing tests of the module you're touching

For Phase 2.6c (TaggedValue) work also read [`docs/design/tagged-semantics.md`](docs/design/tagged-semantics.md) — the Single Source of Truth for slot layout, producer / consumer matrix, runtime invariants (ADR 0068).

Open an issue before non-trivial work; there may already be a direction in mind.

## Setup

Primary dev environment: WSL2 Arch Linux (see [ADR 0005](docs/design/0005-mlir-environment.md)). Working tree lives at `~/LuMeLIR` (native ext4). Anything that pulls `melior` / `mlir-sys` needs WSL2; pure-Rust layers also build on Windows but Windows native MLIR is best-effort only.

Under WSL2 Arch Linux:

- MLIR toolchain: `sudo pacman -S base-devel llvm rust cmake ninja pkgconf clang zlib zstd libxml2` plus `paru -S mlir` (AUR; matches melior 0.27 = MLIR 22).
- Env vars for `melior` (put in `~/.bashrc` or a repo-local script):
  ```bash
  export MLIR_SYS_220_PREFIX=/usr
  export LLVM_SYS_220_PREFIX=/usr
  export TABLEGEN_220_PREFIX=/usr
  ```
- Sanity check: `llvm-config --version` and `mlir-tblgen --version` should both report 22.x.
- If `bindgen` complains about a weird libclang from a Windows host, `unset LIBCLANG_PATH` first.

Windows + Git Bash notes (pure-Rust layers only; do not run MLIR-linked builds here):

- Use Unix shell syntax (`/dev/null`, not `NUL`).
- `/usr/bin/link.exe` (Git Bash) shadows MSVC `link.exe`. WSL2 sidesteps this.
- `/mnt/v/melior-spike/FINDINGS.md` documents prior MSVC port attempts.

## Coding Principles

### Functional Programming First

- Pure functions by default. Keep data flow as `input → pure transform → output`.
- Push side effects (file I/O, stdout, process spawn, allocator choice) to layer boundaries.
- Prefer `Iterator` adapters and `map` / `fold` over mutable accumulators.
- Escape hatch: impurity is permitted when profiling shows it matters (e.g. tokenizer buffer reuse). Justify with a comment *and* an ADR if the API leaks mutation.

Rationale: [ADR 0120](docs/design/0120-engineering-principles-fp-first.md).

### Clean Architecture (Layering)

Dependency direction (outer → inner):

```
cli  →  (lib crate root)  →  codegen  →  mir  →  hir  →  parser  →  lexer
```

- Each layer may only `use` items from layers strictly inside it. Reverse dependencies are forbidden.
- MLIR / Melior / LLVM-sys bindings are confined to the `codegen` layer. `hir` / `mir` use plain Rust types.
- `src/lib.rs` is the library root; `src/main.rs` is a thin entry (<20 lines) calling `lumelir::cli::run()`. See [ADR 0002](docs/design/0002-lib-rs-layering.md).

Rationale: [ADR 0121](docs/design/0121-layering-clean-architecture.md).

### Test-Driven Development

Cycle: **Red → Green → Refactor.**

1. **Red** — write a failing test first. Scope it tightly: `cargo test --lib lexer::tests::lex_integer`.
2. **Green** — write the minimum code to pass.
3. **Refactor** — improve structure while tests stay green.

Commit granularity: one commit per red→green transition is ideal but not enforced; refactor commits stay separate.

Test placement:

- **Unit** (pure logic): at the end of the module file, inside `#[cfg(test)] mod tests { ... }`.
- **Integration** (CLI, file I/O): under `tests/` (e.g. `tests/cli_compile.rs`).
- **Fixtures**: `tests/fixtures/*.lua`.

Test naming: `fn <subject>_<condition>_<expectation>()`. Example: `fn lex_integer_literal_yields_single_number_token()`.

Rationale: [ADR 0122](docs/design/0122-tdd-red-green-refactor.md). Refactor commit separation: [ADR 0123](docs/design/0123-tidyfirst-refactor-discipline.md).

### Rust-Specific

- Lint gate: `cargo clippy --all-targets -- -D warnings` must pass.
- `unwrap` / `expect` are forbidden in non-test code unless justified with a comment explaining why the invariant holds.
- Library layers use `thiserror`-derived enums; the CLI layer may use `anyhow` to collapse them at the boundary. See [ADR 0003](docs/design/0003-error-handling.md).
- `unsafe` requires a `// SAFETY:` comment and is confined to MLIR/LLVM FFI boundaries.
- Avoid `Clone` unless ownership demands it; prefer borrowing.
- No premature abstractions. Follow the "rule of three" before extracting a helper.

Security-relevant rules (`unsafe` confinement, `unwrap`/`expect` discipline) are covered by [ADR 0126](docs/design/0126-security-policy.md).

## Local Gate

Run before pushing:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same gate on every PR — see [ADR 0124](docs/design/0124-ci-cd-policy.md).

## Commits & Pull Requests

### Conventional Commits

Format: `<type>(<optional-scope>): <subject>`

Allowed types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`.

Subject rules: imperative mood, lowercase start, no trailing period, ≤72 chars.

Rationale: [ADR 0130](docs/design/0130-commit-message-convention.md).

### PR Discipline

- One PR = one logical change. If the title contains "and", split it.
- Reference the relevant ADR number in the PR description when a design decision is involved.
- A PR that changes behavior without tests is not mergeable.
- Branch off `main` (`feat/...`, `fix/...`, `docs/...`, `chore/...`, `refactor/...`, `test/...`, `ci/...`).

Rationale: [ADR 0131](docs/design/0131-pr-discipline-code-review.md).

## ADR Workflow

Conventions and chronological index live in [`docs/design/README.md`](docs/design/README.md). Write an ADR when:

- adding a new crate dependency,
- changing module/layer boundaries,
- making a deliberate trade-off between performance and readability/maintainability,
- choosing between two viable implementation strategies.

Filename: `NNNN-kebab-title.md` (zero-padded, monotonic). Reference the ADR number in the PR description and commit messages.

ADR ID vs phase tag: [ADR 0129](docs/design/0129-phase-tag-convention.md). What qualifies as an ADR: see `docs/design/README.md` "When to write an ADR".

## Dependency Addition Policy

Add dependencies at the moment they are first needed, together with an ADR. No speculative additions.

1. Justify in an ADR (alternatives considered, trade-offs).
2. Use the dependency in the same PR that adds it — no placeholder additions.
3. Check `cargo tree` for unexpected transitive dependencies.

Rationale: [ADR 0128](docs/design/0128-dependency-addition-policy.md).

## Security

`unsafe` is confined to MLIR/LLVM FFI boundaries; every `unsafe` block carries a `// SAFETY:` comment. `unwrap` / `expect` outside tests requires justification. Dependencies are reviewed for license, maintenance status, and transitive footprint (`cargo tree`). No secrets in the repository. Vulnerability reports go through GitHub Security Advisories (private).

Full policy: [ADR 0126](docs/design/0126-security-policy.md).

## Releases

Semantic versioning. Phase 2 complete → `v0.1.0`; Phase 3 start → `v0.2.0`; pre-1.0 allows breaking changes at any minor bump. Each release tags `vX.Y.Z` and rotates the `[Unreleased]` section of `CHANGELOG.md` (Keep a Changelog format).

Full policy: [ADR 0125](docs/design/0125-release-procedure.md).

## Documentation Update Policy

- [`docs/PRD.jp.md`](docs/PRD.jp.md) is the Source of Truth. [`docs/PRD.md`](docs/PRD.md) is a best-effort English translation and may drift — keep the footer pointing back to the Japanese SoT.
- [`README.md`](README.md) (English) is primary; [`docs/README.jp.md`](docs/README.jp.md) is the translation.
- When you change a policy in this file, update it in the same commit as the code/ADR change. Stale convention docs are the worst failure mode.

Full policy: [ADR 0127](docs/design/0127-documentation-policy.md).

### TaggedValue SoT update checklist

[`docs/design/tagged-semantics.md`](docs/design/tagged-semantics.md) is the SoT for the Phase 2.6c TaggedValue runtime model (ADR 0068). When a PR touches:

- `src/codegen/emit.rs` TaggedValue dispatch helpers
  (`emit_value_slot_*`, `emit_local_init_tagged`, `emit_isnil_index`,
  `emit_print_tagged_local`, `emit_type_tagged_local`,
  `emit_tostring_tagged_local`, `emit_tagged_eq_*`,
  `emit_tagged_unknown_tag_trap`)
- `src/hir/mod.rs` HIR variants for tagged values
  (`HirExprKind::IndexTagged`, `IsNil`, `ValueKind::TaggedValue`)
- Any test under `tests/phase2_6c_tag_*`

… confirm the SoT doc is up to date. The ADR's *Documentation updates* checklist (per `docs/design/README.md` template) records which sections were touched, or justifies "no change required".

## Licensing

LuMeLIR is dual-licensed under **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE)) and **MIT license** ([LICENSE-MIT](LICENSE-MIT)). Users may choose either.

Unless you state otherwise, any contribution you intentionally submit for inclusion, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.

## For LLM Coding Agents

The conventions above apply to LLM agents the same as humans. LLM-specific safety guardrails (destructive operations, do-not-touch list, commit / push instructions) live in [`AGENTS.md`](AGENTS.md) — load it before making edits.
