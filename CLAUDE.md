# CLAUDE.md

Canonical working conventions for this repository live in **[AGENTS.md](./AGENTS.md)**.
Read that file first. It covers:

- Current phase status (Phase 0 complete, Phase 1 PoC in progress)
- Coding principles (FP / Clean Architecture / TDD)
- Commit, ADR, and dependency policies
- LLM-agent-specific rules (destructive operations, forbidden files, Windows/bash gotchas)

## Claude-Specific Notes

- Do **not** edit `.claude/settings.local.json` — it is user-local and excluded via `.claude/.gitignore`.
- Product requirements Source of Truth: [`docs/PRD.jp.md`](./docs/PRD.jp.md) (Japanese). The English [`docs/PRD.md`](./docs/PRD.md) may drift.
