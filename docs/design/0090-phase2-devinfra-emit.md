# 0090. Phase 2.devinfra-emit: CLI Pipeline-Stage Emission (`--emit <stage>`)

- **Status:** Accepted
- **Date:** 2026-05-10
- **Deciders:** ShortArrow

## Context

ADR 0089 (`09ea6f4`, 2026-05-10) closed `LIC-2.7p-arith-coerce-tagged-1`,
reaching Phase 2 tagged-semantics consumer coverage complete
(28 / 28 / 0). With language semantics stable, the next investment
is *observability*: a way to inspect intermediate compiler artifacts
(HIR / MLIR text / LLVM IR text) without tracing through the codegen
internals.

A user proposal to bundle "container e2e + DAP + dump" was sent for
codex review (6 視点). The verdict was clear:

| Perspective | Container | DAP | Dump |
|---|---|---|---|
| non-ad-hoc / TDD / Docs | Refactor / Refactor / No-go | No-go × 3 | **Go** × 3 |
| FP / CA / Security | Go / Go / Refactor | No-go × 3 | Refactor × 3 |

Codex critical: **do not bundle 3 features**; dump alone is the
surgical pick because (a) it directly accelerates semantic-feature
debugging, (b) it does not conflict with ADR 0005's "no Docker in
Phase 1" stance, (c) it adds zero attack surface vs. DAP's network /
debugger-attach concerns. User confirmed: **dump now, container
after dump, DAP roadmap-only**.

A second codex pass on plan v1 returned **Refactor** with 4 critical
issues:

1. `EmitStage` ownership — must NOT be CLI-local; future DAP / LSP /
   programmatic API need the same enum.
2. `cli::compile::invoke` was directly orchestrating the 3-stage
   pipeline; pipeline knowledge belongs in a use-case layer, not
   the I/O adapter.
3. e2e oracle was `contains(token)` only; should combine **include**
   AND **exclude** layer-specific tokens to detect mis-routing.
4. ADR text needs to distinguish `--emit hir` / `--emit mlir`
   (pure render) from `--emit llvm` (effectful generate via
   `mlir-opt` / `mlir-translate` subprocesses).

Plan v2 (this ADR) addresses all four. Implementation follows the
plan exactly.

## Decision

### `src/pipeline.rs` (new module) — use-case layer

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EmitStage { Hir, Mlir, Llvm }

#[derive(Debug)]
pub enum PipelineArtifact {
    Hir(String),    // pure render: format!("{:#?}", hir)
    Mlir(String),   // pure render: module.as_operation().to_string()
    Llvm(String),   // effectful generate: subprocess(mlir-opt + mlir-translate)
}

pub fn compile_until(source: &str, stage: EmitStage)
    -> Result<PipelineArtifact, anyhow::Error>;
```

Internal helpers (private to `pipeline`):
- `render_hir(&HirChunk) -> String` — pure
- `render_mlir(&Module) -> String` — pure (Context-borrow only)
- `generate_llvm_ir(&Module) -> Result<String>` — effectful;
  wraps existing `codegen::lower::to_llvm_ir`.

Effect boundary explicitly named in code and doc-comments:
**render** for the in-memory→text path (Hir, Mlir),
**generate** for the subprocess-spawning path (Llvm).

### `src/cli/` — thin I/O adapter

- `cli::mod.rs` adds `emit: Option<pipeline::EmitStage>` to the
  `Compile` subcommand. Reuses `clap::ValueEnum` derive on the
  pipeline-owned enum (no CLI-local copy).
- `cli::compile::invoke` branches: when `emit.is_some()`, call
  `compile_until` and pass the artifact text to `write_dump`;
  otherwise the existing parse → lower → `codegen::compile` path.
- `write_dump(text, output)` — stdout default; `-o PATH` writes a
  file. Same `-o` flag the no-emit path already uses; help text
  documents the dual semantics.

### `src/codegen/` — unchanged

The library API `lumelir::codegen::compile()` keeps its existing
signature and behaviour. ADR 0090 introduces no public-API change
in `codegen`. Verified post-implementation:
`git diff --stat src/codegen/` → **0**.

This preserves the CA invariant: codegen exposes the in-memory
artifacts it has always produced; the pipeline layer aggregates
them; the CLI is a thin adapter.

### CLI usage

```text
$ lumelir compile hello.lua --emit hir          # to stdout
$ lumelir compile hello.lua --emit mlir          # MLIR text
$ lumelir compile hello.lua --emit llvm          # LLVM IR text
$ lumelir compile hello.lua --emit mlir -o out.mlir   # to file
$ lumelir compile hello.lua --emit llvm | clang -x ir -o app -
                                                  # alt back-end
$ lumelir compile hello.lua                      # full compile (unchanged)
```

### Diagnostic surface for `--emit llvm`

Unique to the Llvm stage: `mlir-opt` and `mlir-translate` failures
surface as `anyhow::Error` from the subprocess. Failure messages
may include environment-dependent info (binary paths, MLIR /
LLVM versions). This is consistent with the existing full-compile
behaviour where `to_llvm_ir` already invokes the same subprocesses;
ADR 0090 introduces no new attack surface.

The `--emit hir` and `--emit mlir` stages are subprocess-free —
errors come only from the parser / HIR layers and surface via
`anyhow`'s default formatting. The richer `format_error` / source-
snippet diagnostic in the no-emit path is intentionally not yet
plumbed through `compile_until`; a future devinfra ADR can add a
typed `PipelineError` if structured diagnostics on `--emit` paths
become important.

## Phase tag rationale (`2.devinfra-*`)

ADR 0090 introduces a cross-cutting **`2.devinfra-*`** sub-lane in
AGENTS.md's progress table. The numbered language phases (2.6,
2.7, 2.8, …) track Lua semantic features. Dev-infra investments
(observability, container reproducibility, future LSP / DAP) are
recurring but orthogonal to language coverage. A dedicated sub-
lane keeps the AGENTS.md row layout coherent: language-feature
ADRs continue under `2.6+`, dev-infra ADRs collect under
`2.devinfra-*`. Future container and DAP ADRs reuse the tag.

## Alternatives Considered

- **Plan v1: `EmitStage` in `src/cli/`.** Rejected — codex review
  v2 §non-ad-hoc: future DAP / LSP / API consumers would either
  duplicate the enum or pull a CLI dependency for a pipeline
  concept. The pipeline layer is the natural owner.
- **Refactor `codegen::compile()` to take an `Until: EmitStage`
  parameter.** Rejected — would change a public library API for a
  dev-infra feature, and the existing `compile()` is consumed by
  `Commands::Run` and any future programmatic user. v2 keeps it
  unchanged; `compile_until` is the new opt-in entry.
- **Bundle container + DAP + dump.** Rejected by codex v1 critical
  #1 (3 features = 3 separate ADRs); by v1 critical #2 (ADR 0005
  reject reasons unchanged); by v1 critical #3 (DAP attack
  surface). User-confirmed.
- **`--dump-hir` / `--dump-mlir` / `--dump-llvm` boolean flag set
  vs. single `--emit <value>`.** Rejected — combinatorial flag
  hell, no clean stop-point semantics. `--emit` mirrors `rustc
  --emit` and is extensible (future `asm`, `obj`, etc.).
- **Golden-file harness for IR diffs.** Rejected for MVP — would
  freeze IR shape across rustc / MLIR / LLVM updates and create
  brittle fixtures. `contains()` snippet oracle plus exclusion
  checks is robust enough for the layer-routing-correctness
  guarantee this MVP needs.

## Consequences

- **Test totals: 1013 → 1018 green** (5 new e2e: 4 emit-stage
  behaviour + 1 regression-pin).
- **LIC counter unchanged** (28 / 28 / 0). ADR 0090 is dev-infra,
  not language semantics.
- **Source LOC delta**:
  - `src/pipeline.rs` (new): ~100 LOC
  - `src/lib.rs`: +9 LOC (module decl + layering doc)
  - `src/cli/mod.rs`: +9 LOC (emit field + thread-through)
  - `src/cli/compile.rs`: +30 LOC (emit branch + write_dump)
  - **`src/codegen/`: 0 LOC** (CA invariant)
  - tests: ~165 LOC (5 e2e)
- **CA invariant assertion**: `git diff --stat src/codegen/` = 0.

### Future work

- **Container e2e** — re-evaluation deferred to "post-CI
  introduction". ADR 0005 reject reasons (WSL2 sufficient, Docker
  Desktop dependency adds friction) remain valid as of 2026-05-10.
  Container ADR will be filed when CI / multi-contributor on-
  boarding pressure emerges; expected to reuse the
  `2.devinfra-*` phase tag.
- **DAP integration** — roadmap-only at this ADR. Prerequisites:
  (a) source-location metadata in HIR (currently HIR carries
  byte offsets at error sites only, not at instruction granularity);
  (b) a debug-runtime contract design ADR (step / continue /
  breakpoint semantics for the AOT-compiled binary). DAP brings a
  network protocol surface, attach-target surface, and authentication
  questions that are out of scope for an MVP investment. Future
  DAP ADR will reuse the `2.devinfra-*` phase tag.
- **`--emit asm`** — post-LLVM x86 assembly via `llc`; nice-to-have.
- **`--emit mlir-llvm`** — post-`mlir-opt`, pre-`mlir-translate`
  intermediate; useful for melior dialect-lowering debugging.
- **`PipelineError` typed enum** — replace `anyhow::Error` for
  structured diagnostics on `--emit` paths if richer error
  reporting becomes important.
- **`--sanitize-paths` / `--strip-source-info`** — codex security
  Refactor mention; relevant if PR / CTF-publishing workflow
  emerges where leaking source paths in IR is a concern.

## Documentation updates

- [x] **ADR 0090** (this file) authored.
- [x] **`docs/design/tagged-semantics.md`** §8 ADR index — 1 row
  for 0090 (devinfra-emit).
- [x] **`AGENTS.md`** — `‣ 2.devinfra-emit` row added; new
  `2.devinfra-*` sub-lane introduced.
- [x] **`docs/PRD.jp.md`** — 1 paragraph `--emit` usage example.
  DAP roadmap stays in this ADR's Future work section (codex
  nice-to-have #3 — keep PRD scoped to product requirements).
- [x] **`docs/design/0005-mlir-environment.md`** — 1-sentence
  status note: container e2e remains deferred as of ADR 0090
  (2026-05-10); re-evaluation triggers documented here. **Not a
  partial supersede** — ADR 0005's decision is unchanged.

## ADR 0002 (lib-rs-layering) consistency note

ADR 0002 states "Side effects (I/O, process spawn) are confined
to `cli`; inner layers are pure." Current reality: `codegen::lower`
already spawns `mlir-opt` and `mlir-translate`; `codegen::link`
spawns `clang` / `llc`. The principle is partly aspirational —
applies strictly to **compile-time logic** (HIR transforms, MLIR
construction) but not to backend tool invocation.

ADR 0090's `pipeline.rs` is consistent with this reality: its
`generate_llvm_ir` wraps the existing effectful `to_llvm_ir`
(no new escape hatch). The new layering chain reads:

```
cli → pipeline → codegen → hir → parser → lexer
```

`pipeline` has no I/O of its own; it composes existing helpers.
The CLI continues to own user-facing side effects (stdout / file
write via `write_dump`).

## Lua-Incompatibility Tracker

ADR 0090 is dev-infra; no LIC entries are added or modified. See
`docs/design/tagged-semantics.md` §4 for the authoritative LIC list
(28 / 28 / 0 as of ADR 0089).
