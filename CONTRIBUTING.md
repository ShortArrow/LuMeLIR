# Contributing to LuMeLIR

Thanks for your interest! LuMeLIR is in **Phase 0 (scaffolding complete)** with **Phase 1 PoC** (`print(1 + 2)` AOT through MLIR) currently underway.

## Before You Start

- Read [`docs/PRD.jp.md`](docs/PRD.jp.md) (Japanese SoT) or [`docs/PRD.md`](docs/PRD.md) (English translation) for the product vision.
- Browse [`docs/design/`](docs/design/) for Architecture Decision Records (ADRs).
- Open an issue before non-trivial work — there may already be a direction in mind.

## Workflow

1. Fork, branch off `main` (`feat/...`, `fix/...`, `docs/...`, `chore/...`).
2. Make changes. Write tests first when adding behavior (TDD; see [AGENTS.md §4.3](AGENTS.md#43-test-driven-development)).
3. Keep commits small and focused. Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`, `perf:`, `build:`, `ci:`).
4. Run the local gate before pushing:
   ```bash
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
5. Open a PR against `main`. Reference any relevant ADR number in the description.
6. Non-trivial design decisions should land as an ADR under [`docs/design/`](docs/design/) in the same PR.

## Licensing

LuMeLIR is dual-licensed under **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE)) and **MIT license** ([LICENSE-MIT](LICENSE-MIT)). Users may choose either.

Unless you state otherwise, any contribution you intentionally submit for inclusion, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.

## For LLM Coding Agents

Your canonical instructions are in **[AGENTS.md](AGENTS.md)**. Load it before making edits. Key points:

- Do not commit or push without explicit user instruction.
- Do not add crate dependencies for phases that have not yet started.
- Do not touch `.claude/settings.local.json`, `git config`, or `LICENSE-*` files.
- Prefer pure functions; side effects belong at layer boundaries.
- TDD cycle: red → green → refactor.

See [AGENTS.md §10](AGENTS.md#10-llm-agent-specific-rules) for the full list of agent-specific rules.
