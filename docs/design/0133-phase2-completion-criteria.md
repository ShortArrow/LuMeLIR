# 0133. Phase 2 Completion Criteria (Scope Freeze)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

PRD §Phase 2 names "core semantics" but the actual working backlog after ADRs 0007–0119 spreads across multiple workstreams (metatables, `_ENV`, `pcall`, `string.format`, GC, etc.). Without a scope freeze, the next ADR (metatables `__index` — ADR 0134) risks sprawling into "design the whole metaobject protocol".

Codex post-`2226c79` review verdict: write this meta-ADR before ADR 0134 so the scope contract is explicit.

## Decision

Phase 2 closes when each workstream below has an Architecture Decision recorded; **implementation may stagger into 2.7+ sub-phase work** without blocking the close.

**In Phase 2** (this set must land before declaring Phase 2 done):

- Metatables read-path (`__index` Table) — [ADR 0134](0134-metatables-index-read.md).
- GC strategy — TBD ADR (trigger below).

**Deferred to 2.7+** (each gets its own ADR; not blocking Phase 2 close):

| Workstream | Hard trigger / dependency |
|---|---|
| `__newindex` write-path | [ADR 0135](0135-metatables-newindex-write.md) — Table form, hash key only. Function-form and Number-key (array) `__newindex` remain separate ADRs. |
| `__index = Function` form | After [ADR 0134](0134-metatables-index-read.md) + call-ABI cleanup. |
| Arithmetic metamethods (`__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__unm` / `__idiv` / `__band` / `__bor` / `__bxor` / `__bnot` / `__shl` / `__shr`) | After [ADR 0134](0134-metatables-index-read.md); one ADR per op (or per family). |
| Comparison metamethods (`__eq` / `__lt` / `__le`) | After [ADR 0134](0134-metatables-index-read.md). |
| `__tostring` / `__concat` | After [ADR 0134](0134-metatables-index-read.md). Closes the rejection at `src/hir/mod.rs:399`. |
| `__call` | After [ADR 0134](0134-metatables-index-read.md). Closes the future-work note at `src/hir/mod.rs:1731`. |
| `_ENV` / true globals | After `__newindex`; supersedes [ADR 0048](0048-phase2-0a-auto-declare-globals.md). |
| `pcall` / `error` value propagation | After [ADR 0134](0134-metatables-index-read.md) (error-table shape depends on metatables). |
| `string.format` | Parallel side-track; no metatable dependency. |
| `string` patterns (`find` / `match` / `gmatch` / `gsub`) | Parallel side-track; pattern engine is its own design surface. |
| **GC strategy decision** | Before the first heap-allocated metatable lands (i.e. before `__newindex` or any shared-metatable ADR). [ADR 0134](0134-metatables-index-read.md) itself does NOT allocate new heap — `metatable_ptr` lives in the existing table header. |

## Alternatives considered

- **No meta-ADR, write ADR 0134 directly.** Rejected — Codex review flagged scope creep risk; the project has a documented history of ADR-A sprawling without a pre-stated boundary.
- **One omnibus "Phase 2.6+ metaobject protocol" ADR.** Rejected — would conflate read/write/arith/cmp/call into a multi-thousand-line document with no review checkpoint.
- **Skip GC and ship Phase 2 without a memory model decision.** Rejected — the trigger ("before first heap-allocated metatable") would then have nothing to enforce; the deferral table makes the trigger discoverable.

## Consequences

**Positive**
- ADR 0134 has a literal scope ceiling ("read-path Table-only"); any expansion lands as a separate, reviewable ADR.
- GC strategy gets a hard trigger instead of drifting into Phase 3.
- Phase 2 close is an objective ("all rows have ADRs"), not a subjective ("feels done").

**Negative**
- 10+ follow-up ADRs are now scheduled but unwritten. They will appear over Phase 2.7+ work.
- The deferral table itself will need maintenance as workstreams land or get re-scoped.

**Locked in until superseded**
- "Phase 2 close = each workstream has an ADR" is the contract. Adding a new workstream requires updating this table in the same PR.

## References

- [ADR 0048](0048-phase2-0a-auto-declare-globals.md) — original "Phase 2.6+ structural project" note that motivates this scope freeze.
- [ADR 0124](0124-ci-cd-policy.md) — CI gate that protects each future ADR's implementation commit.
- [ADR 0129](0129-phase-tag-convention.md) — foundational ADRs (like this one) carry no phase tag.
- Codex post-`2226c79` review verdict — recommended F+A bundle.
