# AGENTS.md — LuMeLIR Working Conventions for LLM Coding Agents

> Primary audience: LLM coding agents (Claude Code, OpenAI Codex, Cursor, Aider, Devin, ...).
> Human contributors: see [CONTRIBUTING.md](CONTRIBUTING.md) first, then come back here for details.

## 0. About This Document

- **Single source of truth** for working conventions in this repository.
- `CLAUDE.md` and `CONTRIBUTING.md` are thin pointers — do not duplicate content there.
- If this file exceeds ~350 lines, split details into `docs/agents/*.md` and keep this file as the index.
- Update this file in the same commit as any policy change (see §9).

## 1. 30-Second Project Summary

LuMeLIR is a Rust-based compiler toolchain that lowers Lua through **MLIR** into native AOT binaries for heterogeneous targets (CPU / GPU / FPGA / MCU). The thesis: **Lua as a frontend for MLIR's transformation engine**, not merely a scripting language.

Full product requirements: [`docs/PRD.jp.md`](docs/PRD.jp.md) (Source of Truth, Japanese) / [`docs/PRD.md`](docs/PRD.md) (English translation).

## 2. Current Phase Status

| Phase | Status | Scope |
|---|---|---|
| Phase 0 — Scaffolding | **Done** | Cargo workspace, CLI skeleton (clap), docs, dual license, ADR conventions |
| Phase 1 — PoC | **In progress** | `print(1 + 2)` AOT: lexer → parser → HIR → MLIR emit → native binary |
| Phase 2 — Core Semantics | Not started | Tables, metatables, GC strategy |
| Phase 3 — Domain Features | Not started | Rust-Lua inline bridge, embedded register dialect |

**How to read TBD markers:** sections marked `TBD: Phase N, ADR XXXX` indicate the rule is undecided until that ADR lands. Do not invent answers — surface the question instead.

## 3. Required Reading Before You Start

1. [`docs/PRD.jp.md`](docs/PRD.jp.md) — product intent (SoT)
2. [`docs/design/README.md`](docs/design/README.md) — ADR conventions
3. `docs/design/NNNN-*.md` — any ADRs relevant to your task
4. This file
5. Existing tests of the module you're touching

## 4. Coding Principles

### 4.1 Functional Programming First

- **Pure functions by default.** Keep data flow as `input → pure transform → output`.
- Push side effects (file I/O, stdout, process spawn, allocator choice) to layer boundaries.
- Prefer `Iterator` adapters and `map`/`fold` over mutable accumulators.
- **Escape hatch:** impurity is permitted when profiling shows it matters (e.g. tokenizer buffer reuse). Justify with a comment *and* an ADR if the API leaks mutation.
- Examples:
  - Preferred: `fn tokenize(src: &str) -> Result<Vec<Token>, LexError>`
  - Justify-in-ADR: `fn tokenize(&mut self, src: &str)` (internal buffer reuse)

### 4.2 Clean Architecture (Layering)

Dependency direction (outer → inner):

```
cli  →  (lib crate root, Phase 1+)  →  codegen  →  mir  →  hir  →  parser  →  lexer
```

- Each layer may only `use` items from layers **strictly inside it**. Reverse dependencies are forbidden.
- MLIR / Melior / LLVM-sys bindings are confined to the `codegen` layer. `hir` / `mir` use plain Rust types.
- Phase 0 current reality: single bin crate, only `cli` module exists. Phase 1 will introduce `src/lib.rs` and shrink `src/main.rs` to <20 lines. **TBD: ADR 0002 (`lib.rs` layering).**

### 4.3 Test-Driven Development

Cycle: **Red → Green → Refactor.**

1. **Red** — write a failing test first. Scope it: `cargo test --lib lexer::tests::lex_integer`.
2. **Green** — write the minimum code to pass. Ugly is fine.
3. **Refactor** — keep tests green while improving structure.

Commit granularity: one commit per red→green transition is ideal but not enforced; refactor commits stay separate.

Test placement:
- **Unit** (pure logic): at the end of the module file, inside `#[cfg(test)] mod tests { ... }`.
- **Integration** (CLI, file I/O): under `tests/` (e.g. `tests/cli_compile.rs`).
- **Fixtures**: `tests/fixtures/*.lua`.

Test naming convention: `fn <subject>_<condition>_<expectation>()`. Example: `fn lex_integer_literal_yields_single_number_token()`.

### 4.4 Rust-Specific Guidance

- Lint gate: `cargo clippy --all-targets -- -D warnings` must pass.
- `unwrap` / `expect` are **forbidden in non-test code** unless justified with a comment explaining why the invariant holds.
- Error types: **TBD: Phase 1, ADR 0003** (`thiserror` vs hand-rolled enum vs `anyhow` boundary).
- `unsafe` requires a `// SAFETY:` comment and is confined to MLIR/LLVM FFI boundaries.
- Avoid `Clone` unless there is a clear ownership reason; prefer borrowing.
- No premature abstractions. Follow the "rule of three" before extracting a helper.

## 5. Test Conventions (Summary)

- Run locally: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`.
- CI: **TBD: Phase 1** (GitHub Actions expected). When added, the above command sequence is the minimum gate.
- Property-based / fuzz testing: **TBD: Phase 2+** (not yet warranted).

## 6. Commits & Pull Requests

### 6.1 Conventional Commits

Format: `<type>(<optional-scope>): <subject>`

Allowed types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`.

Subject rules: imperative mood, lowercase start, no trailing period, ≤72 chars.

Examples from this repo's history:
- `chore: initial scaffold for LuMeLIR (Rust 2024 edition)`
- `chore: track .claude/.gitignore to share local-settings exclusion rule`

### 6.2 PR Discipline

- One PR = one logical change. If you find yourself writing "and" in the PR title, split it.
- Link the relevant ADR number in the PR description when a design decision is involved.
- A PR that changes behavior without tests is **not mergeable**.

## 7. ADR Workflow

Conventions live in [`docs/design/README.md`](docs/design/README.md). Recap:

- Filename: `NNNN-kebab-title.md` (zero-padded, monotonic).
- Write an ADR when:
  - adding a new crate dependency,
  - changing module/layer boundaries,
  - making a deliberate trade-off between performance and readability/maintainability,
  - choosing between two viable implementation strategies.
- Reference the ADR number in the PR description and in commit messages where helpful.

## 8. Dependency Addition Policy

- **Do not add crates for phases that have not started.** The Phase 0 rule (`clap` only until Phase 1 begins) generalizes: add dependencies at the moment they are first needed, together with an ADR.
- When adding a dependency:
  1. Justify in an ADR (alternatives considered, trade-offs).
  2. Use it in the same PR that adds it — no placeholder additions.
  3. Check `cargo tree` for unexpected transitive dependencies.
- Phase 1 expected additions (gated on ADRs): lexer crate (if not hand-rolled), `melior`, `thiserror`/`anyhow`.

## 9. Documentation Update Policy

- **[`docs/PRD.jp.md`](docs/PRD.jp.md) is the Source of Truth.** [`docs/PRD.md`](docs/PRD.md) is a best-effort English translation and may drift — keep the footer pointing back to the Japanese SoT.
- [`README.md`](README.md) (English) is primary; [`docs/README.jp.md`](docs/README.jp.md) is the translation.
- **When you change a policy in this file, update it in the same commit as the code/ADR change.** Stale AGENTS.md is the worst failure mode.

## 10. LLM-Agent-Specific Rules

### 10.1 Destructive Operations Require Explicit Human Approval

Do **not** run the following without the user explicitly asking:

- `git reset --hard`, `git push --force`, `git branch -D`, `git checkout -- .`, `git clean -fd`, `git rebase -i`
- `rm -rf`, recursive directory moves
- `cargo clean` (usually fine but confirm first)
- Any operation that rewrites published history

### 10.2 Do Not Touch

- `.claude/settings.local.json` — user-local Claude Code settings, excluded via `.claude/.gitignore`.
- `git config` — both repository and global scope are off-limits.
- `LICENSE-APACHE`, `LICENSE-MIT` — licensing text is fixed.
- `Cargo.lock` — do not hand-edit. Let `cargo` regenerate it.

### 10.3 Environment Gotchas (Windows 11 + bash)

- Shell is Git Bash / MSYS2-style. Use Unix syntax: `/dev/null` not `NUL`, forward slashes where possible.
- **cwd may reset between tool invocations.** Prefer absolute paths (`V:/LuMeLIR/...`).
- Line endings: repository is LF. Watch for `^M` in diffs — your editor may be inserting CRLF. **TBD: `.gitattributes` in Phase 1.**
- Cargo cold builds take minutes; set generous timeouts for release builds.

### 10.4 Commits & Pushes Require Explicit Instruction

- Never commit autonomously. Wait for the user to say "commit this" or equivalent.
- Never push without explicit instruction.
- Format commit messages per §6.1.

### 10.5 When in Doubt, Ask

If the task is ambiguous, ask the user before writing code. Blindly guessing at intent produces work that gets thrown away and wastes context. A short question beats a long wrong implementation.

## 11. TBD — Decisions Pending

Replace each entry with an ADR link once the decision lands.

- **Lexer implementation strategy**: hand-written vs `nom` vs `logos` → ADR 0001
- **Library/binary split and layer boundaries**: when and how to introduce `src/lib.rs` → ADR 0002
- **Error handling approach**: `thiserror` vs `anyhow` vs hand-rolled enums, and where each applies → ADR 0003
- **CI configuration**: GitHub Actions workflow for fmt / clippy / test / (future) cross-compile
- **`.gitattributes` / `rustfmt.toml`**: formal line-ending and formatting rules
- **Windows vs WSL2/Linux for MLIR builds**: primary development environment for Phase 1

## 12. References

- [`README.md`](README.md) — English overview
- [`docs/README.jp.md`](docs/README.jp.md) — Japanese overview
- [`docs/PRD.jp.md`](docs/PRD.jp.md) — Product Requirements (SoT)
- [`docs/PRD.md`](docs/PRD.md) — Product Requirements (EN translation)
- [`docs/design/README.md`](docs/design/README.md) — ADR conventions and index
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Human contributor guide
- [`CLAUDE.md`](CLAUDE.md) — Pointer for Claude Code
