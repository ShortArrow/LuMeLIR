# 0120. Engineering Principles: Functional-First

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-23
- **Deciders:** ShortArrow

## Context

LuMeLIR has been written FP-first since Phase 0 (pure parser / lexer, effect at codegen boundary, `Iterator`-driven AST traversals). The practice predates any explicit policy; this ADR codifies it so future contributors and reviewers have a canonical reference.

A compiler frontend is a textbook case for FP discipline: each phase is a transformation from an immutable input data structure to an immutable output. Mixing effectful state across phases would obscure where bugs live. The MLIR builder (melior) is one of the few unavoidable effect points and must stay contained.

## Decision

- **Pure functions by default.** Each layer's public surface is `input → Result<output, error>` with no observable side effects.
- **Effects at layer boundaries.** File I/O, stdout, process spawn, allocator choice, MLIR builder mutation all live in (and only in) the outermost layer that needs them — `cli` for user-facing I/O, `codegen` for MLIR.
- **`Iterator` adapters over mutable accumulators.** `.map().filter().collect()` is preferred over a manually-reset `Vec` push loop where readability is comparable.
- **Impurity needs justification.** A function that takes `&mut self` for a reason that is not "this is the effect boundary" needs a comment explaining why, and an ADR if it leaks through the layer's public API.

## Alternatives considered

- **Heavy OO (visitor-pattern AST traversal with mutable state per node).** Rejected. Lua's AST is small enough that the visitor ceremony costs more than it saves; pattern matching on `enum` variants is more direct than dispatching on `dyn Visitor`. Future complexity (Phase 3 Rust interop) does not change this — the AST is still finite and known.
- **Imperative-first (mutable state freely throughout).** Rejected. The bug class this prevents — "did the parser leave state behind that affected codegen?" — is exactly the class a compiler frontend must defend against. The cost of FP discipline is paid once at design time; the cost of mutable-state bugs is paid every time a developer reads the code.
- **No policy (let each contributor choose).** Rejected. Inconsistency in this dimension is the worst outcome; you cannot reason about a half-FP, half-imperative module.

## Consequences

**Positive**
- Each phase's tests are pure unit tests with no fixture or setup boilerplate.
- Refactoring a layer's internals does not risk leaking state to another layer.
- The codegen layer (where `melior` requires eager region build with internal mutation) is the *only* layer where contributors must reason about effect ordering — and it is documented as such.

**Negative**
- `Clone` shows up more often than in idiomatic Rust. We accept this in non-hot paths; in hot paths we justify `&` vs `Clone` with a comment.
- The melior builder API does not compose with iterators; codegen has visible imperative shape. ADR 0121 (layering) explicitly carves out this boundary.

**Locked in until superseded**
- Removing the layer/effect boundary would require an ADR replacing both this and ADR 0121.

## References

- `CONTRIBUTING.md` §3.1 "Functional Programming First" — current rules.
- ADR 0121 (layering) — defines where effect boundaries actually live.
- melior 0.27 API — the eager-region-build constraint that makes codegen the natural effect boundary.
