# CLAUDE.md

Canonical working conventions live in **[CONTRIBUTING.md](./CONTRIBUTING.md)** — read that first (coding principles, TDD, commits, ADR workflow, dependency policy, setup).

LLM-agent-specific safety rules layered on top live in **[AGENTS.md](./AGENTS.md)** — also required reading (destructive ops, do-not-touch list, commit / push instructions).

## Claude-Specific Notes

- Do **not** edit `.claude/settings.local.json` — it is user-local and excluded via `.claude/.gitignore`.
- Product requirements Source of Truth: [`docs/PRD.jp.md`](./docs/PRD.jp.md) (Japanese). The English [`docs/PRD.md`](./docs/PRD.md) may drift.
