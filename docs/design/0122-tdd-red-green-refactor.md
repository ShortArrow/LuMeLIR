# 0122. TDD: Red-Green-Refactor

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

A compiler is observable only through behaviour — given Lua source X, the produced binary must do Y. There is no "internal state inspection" that meaningfully proves correctness; the only way to know the codegen pipeline still produces correct output for a feature is to run a test.

Phase 0 onwards adopted test-first practice (1260+ tests at the time of this ADR). This works has not been recorded as a decision — this ADR codifies it.

## Decision

- Every feature addition follows **Red → Green → Refactor**:
  1. **Red** — write a failing test scoped tightly (`cargo test --lib lexer::tests::lex_integer`, `cargo test --test phase2_stdlib_io`, etc.).
  2. **Green** — write the minimum code to pass.
  3. **Refactor** — improve structure while tests stay green. Refactor commits stay separate (see ADR 0123).
- **Test placement:**
  - **Unit** (pure logic): at the end of the module file, inside `#[cfg(test)] mod tests { ... }`.
  - **Integration** (CLI, file I/O, end-to-end compile + run): under `tests/` (e.g. `tests/phase2_stdlib_io.rs`).
  - **Fixtures**: `tests/fixtures/*.lua`.
- **Test naming:** `fn <subject>_<condition>_<expectation>()`, e.g. `fn lex_integer_literal_yields_single_number_token()`.
- A PR that changes behaviour without a corresponding test is **not mergeable**.

## Alternatives considered

- **Test-after** (write code first, write tests when "ready"). Rejected. Regression detection lags; the test then risks just confirming whatever the code does rather than what was intended. The cost of a bug detected during "manual testing" or "by users" is much higher than catching it at the Red step.
- **No testing** (rely on type checker + manual smoke). Rejected. Type checks catch about 10% of compiler-frontend bugs (the rest are semantic — wrong AST shape, wrong emit pattern, wrong runtime trap). A compiler must demonstrate correctness through behaviour.
- **End-to-end tests only** (skip unit tests, rely on `tests/`). Rejected for hot iteration paths. Integration tests cost seconds per run (MLIR build + native link + execution); unit tests cost milliseconds. Both are needed.

## Consequences

**Positive**
- 1260+ test corpus that compounds over time — every fix gets a regression pin.
- Refactors are safe because the corpus catches regressions.
- New contributors / LLM agents can run the suite to validate any change before commit.
- The "Red Day 0" practice (write the failing test on a separate commit when feasible) creates an audit trail for what each feature was supposed to do.

**Negative**
- CI time grows with the corpus (currently dominated by MLIR build, not test execution).
- Writing a feature is slower in wall-clock time per change — paid back many times over by catching regressions.
- Some features are hard to test (CLI ergonomics, error message wording). For these we accept lighter coverage but still write at least one test per error variant.

**Locked in until superseded**
- The Red → Green → Refactor cycle, the test-included PR rule, and the unit/integration placement convention are baseline. Adjustments (e.g. property-based testing, fuzz testing) extend rather than replace this.

## References

- `CONTRIBUTING.md` §3.3 "Test-Driven Development" — current rules.
- `CONTRIBUTING.md` "Local Gate" — the `fmt && clippy && test` triad enforces this on every PR.
- Kent Beck, *Test-Driven Development by Example*.
- ADR 0123 (TidyFirst) — pairs naturally: refactor commits stay separate so the test corpus stays as the safety net.
