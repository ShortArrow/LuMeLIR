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

- Metatables read-path (`__index` Table) — **RESOLVED by [ADR 0134](0134-metatables-index-read.md) (2026-05-31)**.
- GC strategy — **RESOLVED by [ADR 0145](0145-gc-strategy.md) (2026-05-31)** as decision-only (Phase 2 = leak; Phase 3 = mark-and-sweep with 1 MB trigger).

**Status (2026-06-09): Phase 2 closed.** All in-scope workstreams above have ADRs recorded. Phase 2.7+ continues as incremental sub-phase work; the deferral table below now lists workstreams in their RESOLVED state, retained for historical traceability.

**Deferred to 2.7+** (each gets its own ADR; not blocking Phase 2 close):

| Workstream | Hard trigger / dependency |
|---|---|
| `__newindex` write-path | **RESOLVED**: Table form [ADR 0135](0135-metatables-newindex-write.md); Function form [ADR 0151](0151-newindex-function-form.md); Number-key Table-form [ADR 0168](0168-newindex-number-key-table-form.md); Number-key Function-form [ADR 0169](0169-newindex-function-form-number-key.md); Number-key multi-hop [ADR 0170](0170-multi-hop-number-key-newindex.md); Number-key mid-array `TAG_NIL` trigger [ADR 0171](0171-newindex-mid-array-nil-trigger.md). |
| `__index = Function` form | **RESOLVED by [ADR 0150](0150-index-function-form.md) (2026-05-31)**. |
| Arithmetic metamethods (`__add` / `__sub` / `__mul` / `__div` / `__mod` / `__pow` / `__unm` / `__idiv` / `__band` / `__bor` / `__bxor` / `__bnot` / `__shl` / `__shr`) | **RESOLVED**: arith [ADR 0147](0147-arith-metamethods.md), bitwise [ADR 0148](0148-bitwise-metamethods.md). |
| Comparison metamethods (`__eq` / `__lt` / `__le`) | **RESOLVED by [ADR 0144](0144-comparison-metamethods.md) (2026-05-31)**. |
| `__tostring` / `__concat` | **RESOLVED**: `__tostring` [ADR 0142](0142-tostring-metamethod.md), `__concat` [ADR 0143](0143-concat-metamethod.md). |
| `__call` | **RESOLVED by [ADR 0146](0146-call-metamethod.md) (2026-05-31)**. |
| `_ENV` / true globals | **RESOLVED by [ADR 0154](0154-env-true-globals-strategy.md) (2026-06-01)** as Phase 3 trigger (decision-only). |
| `pcall` / `error` value propagation | **RESOLVED by [ADR 0153](0153-pcall-error-strategy.md) (2026-06-01)** as Phase 3 trigger (decision-only). |
| `string.format` | **RESOLVED by [ADR 0152](0152-string-format.md) (2026-06-01)** (minimum-scope `%d` / `%f` / `%s` / `%%`). |
| `string` patterns (`find` / `match` / `gmatch` / `gsub`) | **RESOLVED by [ADR 0155](0155-string-patterns-strategy.md) (2026-06-01)** as Phase 3 trigger (decision-only). |
| **GC strategy decision** | **RESOLVED by [ADR 0145](0145-gc-strategy.md) (2026-05-31)**: Phase 2 = leak; Phase 3 = non-moving mark-and-sweep with 1 MB trigger. Decision-only ADR; implementation in a future Phase 3 ADR. |

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
