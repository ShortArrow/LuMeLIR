## Summary

<!-- 1-2 sentences. What does this PR do? -->

## Type of change

<!-- Pick one. Matches the Conventional Commits prefix you'll use. -->

- [ ] `feat` — new user-visible behavior
- [ ] `fix` — bug fix
- [ ] `refactor` — behavior-preserving (per [ADR 0123](../docs/design/0123-tidyfirst-refactor-discipline.md))
- [ ] `docs` / ADR
- [ ] `test` — tests only
- [ ] `ci` / `build` — CI / build system
- [ ] `chore` — maintenance
- [ ] **Trivial** (typo, comment, formatting; skip the checklist below)

## ADR reference

<!-- e.g. "Implements ADR 0124." or "N/A — trivial". -->

## Test plan

<!-- Mark trivial PRs as N/A for this whole section. -->

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] Manual smoke / new test cases added (describe):

## Docs updated

<!-- CONTRIBUTING.md / AGENTS.md / CHANGELOG.md / docs/design/* / rustdoc
     touched in this PR? List the files or "N/A". -->

---

See [CONTRIBUTING.md](../CONTRIBUTING.md) for full conventions. ADR criteria self-test lives in [`docs/design/README.md`](../docs/design/README.md) → "When to write an ADR".
