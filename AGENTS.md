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
| Phase 1 — PoC | **Done** | `print(1 + 2)` AOT: lexer → parser → MLIR emit → native binary (ADR 0006) |
| Phase 2 — Core Semantics | **In progress** | `local`, scopes, control flow, tables, metatables, GC |
| ‣ 2.0 `local` + multi-stmt | **Done** | HIR layer introduced; `local x = 1; print(x + 2)` (ADR 0007) |
| ‣ 2.0a auto-declare globals (top-level only) | **Done** | bare `x = 1` at chunk scope auto-declares as chunk-level local; type-stable; no cross-function leak (ADR 0048) |
| ‣ 2.1 reassignment / scopes | **Done** | `x = 2`, `do ... end` blocks, scope stack, shadowing (ADR 0008) |
| ‣ 2.1a multi-target reassignment | **Done** | `a, b = b, a` parallel evaluation via temp-then-assign; auto-declare via ADR 0048 (ADR 0049) |
| ‣ 2.1b multi-target reassign from Call | **Done** | `a, b = pair()` reuses `MultiAssignFromCall` HIR node; targets auto-declare per ADR 0048 (ADR 0050) |
| ‣ 2.2a arithmetic operators | **Done** | `-` `*` `/` `%` `^` + unary `-`; libm pow/floor (ADR 0009) |
| ‣ 2.2b comparisons + bool literals | **Done** | `<` `<=` `==` `~=` `>` `>=`, `true`/`false`; ordered cmpf, print(bool) (ADR 0010) |
| ‣ 2.2c floor div + bitwise ops | **Done** | `//`, `&`/`\|`/`~`/`<<`/`>>`, unary `~`; f64↔i64 via fptosi/sitofp (ADR 0022) |
| ‣ 2.2d hex / float / scientific literals | **Done** | `0xff`, `3.14`, `1e3`, `2.5e-1`; lexer-only change (ADR 0023) |
| ‣ 2.3a nil + per-slot types + heterogeneous == | **Done** | `nil`, `local b = true`, `1 == nil` → false (ADR 0011) |
| ‣ 2.3b control flow | **Done** | `if`/`elseif`/`else`/`while` via `scf`, truthiness helper (ADR 0012) |
| ‣ 2.3c short-circuit | **Done** | `and`/`or`/`not` via `scf.if` expression form + `arith.xori` (ADR 0013) |
| ‣ 2.3d numeric for | **Done** | `for i=s,e[,step] do ... end` via `scf.while` desugar + read-only loop var (ADR 0014) |
| ‣ 2.4 break | **Done** | `break` via HIR-time desugar to hidden `_broken` flag + body guard wrap (ADR 0015) |
| ‣ 2.4b `repeat ... until` | **Done** | do-while loop, until-cond sees body locals, scf.while body-in-`before` (ADR 0035) |
| ‣ 2.5a top-level functions | **Done** | `local function`, `return`, recursion (Number-only params/ret) (ADR 0016) |
| ‣ 2.5b anonymous + first-class (HIR-time) | **Done** | `local f = function() end`, alias `local g = f`, static dispatch (ADR 0017) |
| ‣ 2.5b.2 functions as args | **Done** | `apply(f, x)`, `func.call_indirect`, param-kind back-inference (ADR 0018) |
| ‣ 2.5b.3 functions as return values | **Done** | `return f`, ret_kind→Function, ptr-slot+ucast bridging (ADR 0019) |
| ‣ 2.5e Bool/Nil params/return | **Done** | predicates (`return x > 0`), `not b`, `nil`-returning helpers; call-site param inference (ADR 0020) |
| ‣ 2.5f nested `local function` (no capture) | **Done** | sibling forward-reference + recursion via shared register/lower helpers (ADR 0036) |
| ‣ 2.5c-min capture-by-value closures | **Done** | Number upvalues, direct-call only, MLIR signature widens to `[params + upvalues]` (ADR 0037) |
| ‣ 2.5c.1 top-level `local function` captures chunk locals | **Done** | Pass 2 interleaved with main chunk walk; `idx_of_funcdef` removed (ADR 0042) |
| ‣ 2.5c.2 Bool / Nil / String upvalue captures | **Done** | one predicate flip in `lookup_or_capture_upvalue`; codegen unchanged (ADR 0043) |
| ‣ 2.5c.3 closure-escape static rejection | **Done** | `HirError::ClosureEscapes` for closures-with-upvalues used as args / returns (ADR 0044) |
| ‣ 2.9a line/column diagnostics | **Done** | CLI renders errors as `path:line:col: <layer> error: …` via `cli::diag` (ADR 0045) |
| ‣ 2.9b source-snippet caret display | **Done** | `format_error` appends a rustc-style two-line snippet via `snippet` pure helper (ADR 0046) |
| ‣ 2.9c strip offset from error Display | **Done** | Tidy First — drop redundant `at byte offset N` / `(offset N)` from variant Display strings (ADR 0047) |
| ‣ 2.5d multi-return | **Done** | `return a, b`, `local x, y = call()`, parallel binding, multi-result `func.call` (ADR 0021) |
| ‣ 2.7a string literals + `#` | **Done** | `"..."`/`'...'`, basic escapes, `print(s)`, `#s` via strlen, deduped LLVM globals (ADR 0024) |
| ‣ 2.7b string concat / equality | **Done** | `a..b` via malloc+memcpy, `s1 == s2` via strcmp, call-site String inference (ADR 0025) |
| ‣ 2.7c `tostring` + concat auto-coerce | **Done** | `tostring(x)` builtin, `"x"..1`/`..true`/`..nil` desugar via tostring (ADR 0026) |
| ‣ 2.7d lexicographic string compare | **Done** | `<` `<=` `>` `>=` for String operands via strcmp (ADR 0027) |
| ‣ 2.7e `tonumber` (NaN sentinel) | **Done** | `tonumber(n)` identity, `tonumber(s)` via sscanf, NaN on parse fail (ADR 0028) |
| ‣ 2.7f `type(x)` | **Done** | static kind→typename ptr, Function values admissible (ADR 0029) |
| ‣ 2.7g `assert(cond)` | **Done** | Bool-only assert with libc exit(1) on failure (ADR 0030) |
| ‣ 2.7m `assert(cond, msg)` | **Done** | optional 2nd-arg String message routed into the failure printf (ADR 0051) |
| ‣ 2.7n `tostring(f)` for Function | **Done** | returns the literal `"function"` via shared `s_typename_function` global (ADR 0052) |
| ‣ 2.7h `error(msg)` | **Done** | unconditional failure via shared `emit_exit_with_message` helper (ADR 0033) |
| ‣ 2.8a single-line comments | **Done** | `-- ...` skipped by lexer (ADR 0031) |
| ‣ 2.8b variadic `print` | **Done** | `print()`/`print(a)`/`print(a, b, ...)` with `\t` separator + `\n` (ADR 0032) |
| ‣ 2.8c block comments | **Done** | `--[[ ... ]]` multi-line via `skip_block_comment` helper (ADR 0034) |
| ‣ 2.8d `#!` shebang line | **Done** | leading `#!` skipped to first newline at byte 0 (ADR 0041) |
| ‣ 2.7j long-bracket strings + level-N block comments | **Done** | `[==[ ... ]==]` and `--[==[ ... ]==]` via shared `scan_long_bracket_body` (ADR 0038) |
| ‣ 2.7k extended string escapes | **Done** | `\a \b \f \v \xHH \ddd` (ASCII range) via `read_hex_escape` / `read_decimal_escape` (ADR 0039) |
| ‣ 2.7l `\u{XXXX}` + `\z` | **Done** | Unicode codepoint → UTF-8 via `read_unicode_escape`; `\z` skips whitespace run (ADR 0040) |
| ‣ 2.5c closures | Not started | upvalue capture, heap-allocated environments |
| ‣ 2.6a-min empty tables `{}` + `#t` | **Done** | `ValueKind::Table` (`!llvm.ptr`), `[i64 length]` heap header, malloc on construct (ADR 0053) |
| ‣ 2.6a-arr Number array constructor + `t[i]` read | **Done** | `{e1,e2,…}` populated form, integer indexing, runtime OOB trap (ADR 0054) |
| ‣ 2.6a-wr Number array element write `t[i] = v` | **Done** | parse-then-equals fallthrough, `IndexAssign` AST/HIR, codegen mirrors read path (ADR 0055) |
| ‣ 2.6a-norm stable table header (Tidy First) | **Done** | 32-byte header + separate array_buf; frozen offsets at 0 (length) / 16 (array_buf); alias-safe under grow (ADR 0056) |
| ‣ 2.6a-grow array push `t[#t+1] = v` | **Done** | doubling capacity + realloc inside stable header; alias-safe under grow; LIC-2.6a-wr-2 resolved (ADR 0057) |
| ‣ 2.6b-hash string-keyed `t.k` / `t["k"]` | **Done** | open addressing + linear probing on `hash_buf`; FNV-1a hash; doubling rehash; sugar parser-level (ADR 0058) |
| ‣ 2.6c-tag-arr tagged array slots + holes | **Done** | 16-byte `{tag, value}` slots; `t[#t+2]=v` hole creation with Nil-tagged gap fill; LIC-2.6a-wr-1 resolved (ADR 0059) |
| ‣ 2.6c-tag-hash tagged hash entries + `t.k = nil` | **Done** | 24-byte hash entries (`{ptr key, 16-byte value slot}`); soft-delete via Nil tag; LIC-2.6b-hash-2 partial (Nil accepted) (ADR 0060) |
| ‣ 2.6+ tables / metatables | In progress | hetero values, locals widening, hard tombstone, methods, metatables — multiple sub-phases |
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
- Phase 1 adopted layering: `src/lib.rs` as the library root, `src/main.rs` as a thin entry (<20 lines) calling `lumelir::cli::run()`. See [ADR 0002](docs/design/0002-lib-rs-layering.md).

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
- Error types: library layers use `thiserror`-derived enums; the CLI layer may use `anyhow` to collapse them at the boundary. See [ADR 0003](docs/design/0003-error-handling.md).
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

### 10.3 Environment Gotchas

**Primary dev environment as of Phase 1 MLIR work: WSL2 Arch Linux (see [ADR 0005](docs/design/0005-mlir-environment.md)).** The Rust crate builds on both Windows and WSL2; anything that pulls `melior` / `mlir-sys` needs WSL2.

Under Windows 11 + Git Bash (historical scaffolding env, still usable for pure-Rust layers):
- Shell is Git Bash / MSYS2-style. Use Unix syntax: `/dev/null` not `NUL`, forward slashes where possible.
- **cwd may reset between tool invocations.** Prefer absolute paths (`V:/LuMeLIR/...`).
- Line endings: repository is LF. Watch for `^M` in diffs — your editor may be inserting CRLF. **TBD: `.gitattributes` in Phase 1.**
- Cargo cold builds take minutes; set generous timeouts for release builds.
- `/usr/bin/link.exe` (Git Bash) shadows MSVC `link.exe`; affects any native link step. WSL2 sidesteps this.

Under WSL2 Arch Linux:
- Source tree lives at `/mnt/v/LuMeLIR` (Windows `V:/LuMeLIR/` shared). File I/O is slower than an ext4 home directory — acceptable for now; if build times become a problem, clone a pure ext4 copy into `~/LuMeLIR`.
- MLIR toolchain: `sudo pacman -S base-devel llvm rust cmake ninja pkgconf clang zlib zstd libxml2` plus `paru -S mlir` (AUR).
- Env vars for `melior`: `MLIR_SYS_220_PREFIX=/usr` etc. See `docs/handover/phase1-wsl2-migration.md` for the full bootstrap script.

### 10.4 Commits & Pushes Require Explicit Instruction

- Never commit autonomously. Wait for the user to say "commit this" or equivalent.
- Never push without explicit instruction.
- Format commit messages per §6.1.

### 10.5 When in Doubt, Ask

If the task is ambiguous, ask the user before writing code. Blindly guessing at intent produces work that gets thrown away and wastes context. A short question beats a long wrong implementation.

## 11. TBD — Decisions Pending

Replace each entry with an ADR link once the decision lands.

- **CI configuration**: GitHub Actions workflow for fmt / clippy / test / (future) cross-compile
- **`.gitattributes` / `rustfmt.toml`**: formal line-ending and formatting rules
- **MLIR dialect ownership**: which layer owns FFI, dialect registration, first op set for Phase 1 (ADR pending once the first real codegen lands under WSL2)
- **Windows native MLIR support**: re-opening after ADR 0005 — tracked out-of-tree in `V:/melior-spike/FINDINGS.md`; returns as a future ADR once upstream tblgen accepts the patches

### Resolved
- Lexer implementation → [ADR 0001](docs/design/0001-lexer-implementation.md) (hand-written)
- Library/binary split → [ADR 0002](docs/design/0002-lib-rs-layering.md) (`lib.rs` + thin `main.rs`)
- Error handling → [ADR 0003](docs/design/0003-error-handling.md) (`thiserror` / `anyhow` boundary)
- Parser implementation → [ADR 0004](docs/design/0004-parser-implementation.md) (recursive descent + Pratt)
- MLIR integration environment → [ADR 0005](docs/design/0005-mlir-environment.md) (WSL2 Arch primary, Windows native best-effort)

## 12. References

- [`README.md`](README.md) — English overview
- [`docs/README.jp.md`](docs/README.jp.md) — Japanese overview
- [`docs/PRD.jp.md`](docs/PRD.jp.md) — Product Requirements (SoT)
- [`docs/PRD.md`](docs/PRD.md) — Product Requirements (EN translation)
- [`docs/design/README.md`](docs/design/README.md) — ADR conventions and index
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Human contributor guide
- [`CLAUDE.md`](CLAUDE.md) — Pointer for Claude Code
