# 0130. Commit Message Convention (Conventional Commits)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

The repository has used [Conventional Commits](https://www.conventionalcommits.org) since Phase 0 (`chore: initial scaffold for LuMeLIR (Rust 2024 edition)` — the first commit). The convention has not been recorded as an explicit decision. With ADR 0125 (release procedure) introducing `CHANGELOG.md`, the link between commit format and changelog automation becomes load-bearing — formalizing this convention is now necessary.

## Decision

### Format

```
<type>(<optional-scope>): <subject>

<optional body>

<optional footer>
```

### Type

| Type | Use |
|---|---|
| `feat` | New feature visible to users. |
| `fix` | Bug fix. |
| `chore` | Maintenance task not affecting behaviour (gitignore, lockfile bumps). |
| `docs` | Documentation only (including ADR additions). |
| `test` | Test-only changes (adding regression pins, refactoring tests). |
| `refactor` | Behaviour-preserving code reshuffle (see ADR 0123). |
| `perf` | Performance improvement, behaviour-preserving. |
| `build` | Build system or external dependency changes (`Cargo.toml`, `Cargo.lock`). |
| `ci` | CI configuration changes. |

### Scope

Optional, parenthesized after the type. Examples: `feat(hir): ...`, `docs(adr): ...`, `fix(codegen): ...`. Multi-scope is comma-separated when truly multi-scope: `feat(hir,codegen,docs): ...`. Prefer single-scope; multi-scope is a smell that the PR might want splitting (see ADR 0131).

### Subject rules

- **Imperative mood**: "add", "fix", "remove" (not "added", "fixes", "removed").
- **Lowercase start**: `feat(hir): add ...`, not `feat(hir): Add ...`. Exception: proper nouns (`feat(ci): use GitHub Actions`).
- **No trailing period.**
- **≤72 characters.** Body wraps separately.

### Body

Free-form. Recommended structure:
- One blank line after the subject.
- Paragraphs separated by blank lines.
- Lines wrap at ~72 chars (manual, no enforcement).
- Reference ADRs in the body: `... (ADR 0124)`. Reference PRs / issues / commits as needed.
- Co-author lines (`Co-authored-by: ...`) go in the footer.

### ADR references

When a commit implements or is governed by an ADR, reference it in the body:

```
feat(codegen): add string.char(...) chokepoint

Implements per ADR 0113. New emit_check_byte_arg helper validates
range + integer before f2i; first new producer for the boxed
string ABI (ADR 0112).
```

## Alternatives considered

- **Gitmoji** (`:sparkles: add new feature`). Rejected: shell / log tooling does not parse it reliably; the human-readable type is more discoverable than an emoji lookup.
- **Free-form messages** (no convention). Rejected: changelog automation (ADR 0125) needs a parseable structure. Free-form also makes `git log` navigation harder.
- **Angular-style with mandatory scope.** Considered. Reject mandatory scope: some changes are genuinely cross-scope (e.g. workspace-wide formatting) and inventing a scope adds noise. Optional scope is the right balance.
- **Stricter type vocabulary** (e.g. forbid `chore`). Rejected: `chore` is useful for maintenance commits that don't fit the other types; the cost of misuse is low.

## Consequences

**Positive**
- `git log --oneline` is scannable: type prefix tells you which class of change it is.
- Changelog automation can group by type (`feat` → "Added", `fix` → "Fixed", `refactor` → "Changed", etc.) per Keep a Changelog convention (ADR 0125).
- Multi-LLM-agent collaboration benefits from a structured format; agents can produce conformant messages with high reliability.

**Negative**
- Authors must remember the type vocabulary. Mitigation: `CONTRIBUTING.md` lists them; PR template (future, ADR 0131) reminds.
- A commit that genuinely spans multiple types (e.g. `feat` + `docs` + `test` for one new feature) picks the dominant type. Body explains the rest.

**Locked in until superseded**
- Conventional Commits format is baseline.
- The type vocabulary listed above is baseline; adding a new type requires updating this ADR and `CONTRIBUTING.md` together.

## References

- `CONTRIBUTING.md` "Conventional Commits" — current rules.
- [conventionalcommits.org](https://www.conventionalcommits.org).
- [Angular commit message convention](https://github.com/angular/angular/blob/main/CONTRIBUTING.md#commit) — origin of Conventional Commits.
- ADR 0125 (release procedure) — changelog automation depends on this format.
- ADR 0123 (TidyFirst) — defines `refactor:` semantics.
- ADR 0131 (PR discipline) — future PR template will check commit format.
