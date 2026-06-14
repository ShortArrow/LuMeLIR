# 0190. Phase 2 Epilogue Items — Phase 4 Deferral Decisions

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0189](0189-phase3-entry-criteria.md) §Phase 3 close criteria §3 requires that each Phase 2 epilogue item be resolved (implemented OR explicit deferral to Phase 4) before Phase 3 can close. The four items are:

1. GC mark + sweep + stack walk (v1 safety-mode landed via ADRs 0184-0186; actual freeing not yet implemented)
2. `pcall` / `error` value propagation (designed in ADR 0153; not implemented)
3. `_ENV` / true globals (designed in ADR 0154; not implemented)
4. `string` patterns — `find` / `match` / `gmatch` / `gsub` (designed in ADR 0155; not implemented)

Each was deferred during Phase 2 because the surface area exceeded what the Phase 2 close window allowed. They are PRD Phase 2 in spirit (core language semantics) but were sequenced post-close in practice. This ADR records the disposition for each so ADR 0189 §3 is satisfied.

## Decision

### Per-item disposition

| Item | Disposition | Rationale |
|---|---|---|
| GC actual freeing (stack walk + DFS lift of safety mode) | **Deferred to Phase 4** | ADRs 0184-0186 ship the structural machinery; the v1 safety mode preserves Phase 2 leak semantics. Lifting safety mode requires per-frame alloca-slot registration (ADR 0160 scope) which is large and not on the Rust-Lua Bridge critical path. Phase 4 picks this up alongside performance-oriented optimisation passes (where escape analysis can reduce GC pressure too). |
| `pcall` / `error` value propagation | **Deferred to Phase 4** | ADR 0153 pinned the strategy; implementation is non-trivial (HIR error-propagation arm, codegen runtime helper, error-value box). Not on the Bridge MVP path — the first Rust-Lua Bridge cut uses `f64 → f64` signatures only. Phase 4 lands `pcall` when a Bridge sub-piece needs Rust panic → Lua error surfacing. |
| `_ENV` / true globals | **Deferred to Phase 4** | ADR 0154 pinned the strategy (Phase 3 trigger, decision-only). The current "auto-declared globals" approach (ADR 0048) is functionally adequate for the Bridge POC. Phase 4 lands true `_ENV` when a Bridge consumer needs to read mutated globals across Rust boundaries. |
| `string` patterns (`find` / `match` / `gmatch` / `gsub`) | **Deferred to Phase 4** | ADR 0155 pinned the strategy (Phase 3 trigger, decision-only). String patterns are a self-contained stdlib feature with no dependency on Bridge or GC. Phase 4 lands them when a user-facing demand emerges. |

### Why all four defer

Each item is **independent of the Rust-Lua Bridge minimum cut** (ADR 0191):

- Bridge MVP signature: `extern "C" fn (f64, ...) -> f64` — no error propagation, no globals access, no string patterns, no GC-managed values in the signature.
- GC v1 safety mode preserves Phase 2 leak semantics. Bridge POC programs are short-lived enough that leakage is not user-visible.
- Bridge sub-pieces (ADR 0192+) that need any of these items will trigger their implementation at that time as a Phase 4 work item, not a Phase 3 blocker.

### Phase 3 close criterion §3 — satisfied

With this ADR, each Phase 2 epilogue item has an "explicit deferral to Phase 4" decision recorded. ADR 0189 §3 is satisfied.

### Phase 4 entry signal

When the first Phase 4 work item (any of the four above OR optimisation passes for the PRD success metrics) is scheduled, a future ADR will write the Phase 4 entry criteria (mirror of ADRs 0133 and 0189 for the next phase). Not in scope here.

## Alternatives considered

- **Implement one or more items inside Phase 3.** Rejected — none is on the Bridge MVP critical path; landing them inflates the Phase 3 close window past the project's stated kimo (PRD memo). The disposition is "defer unless triggered", and currently nothing in Phase 3 triggers them.
- **Bundle into a single "Phase 4 lands all four" mega-ADR.** Rejected — each item has independent ADRs (0153, 0154, 0155 strategies; 0156-0162 GC; 0184-0186 partial impl). A single deferral note here is the smallest-touch resolution.
- **Treat the GC v1 safety mode as "implemented" and not defer.** Rejected — observable behaviour (`collectgarbage()` returns 0, `count` accumulates monotonically) is not Lua-spec-conformant. Documenting the gap as "deferred" is honest; "implemented" would mislead.

## Consequences

**Positive**
- ADR 0189 §3 satisfied with one ADR + one commit.
- Phase 3 close path becomes: ADR 0190 (this) → ADR 0191 (Bridge) → ADR 0192 (Embedded decision) → close.
- Phase 4 scope is implicitly seeded with these four items + PRD performance metrics.

**Negative**
- Compiled binaries leak under `collectgarbage()` per v1 safety mode. Documented; not a regression from Phase 2.
- `pcall`, `_ENV`, string patterns remain unimplemented. Documented; not a regression.

**Locked in until superseded**
- "Phase 3 ships v1 safety-mode GC + auto-declared globals; full GC + true `_ENV` + pcall + string patterns ship in Phase 4" is the contract.

## Documentation updates

- [x] §8 — adds 0190.
- [x] ADR 0189 §3 — now satisfied; cross-reference here.

## Test count delta

```
Step 0: 1427 (after e3e1b27)
C1 (this doc): 1427 → 1427 (decision-only, no test)
```

## Critical files

- `docs/design/0190-phase2-epilogue-close-decisions.md` (this doc).
- `docs/design/README.md` index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Bridge sub-piece needs deferred item (e.g. error propagation requires `pcall`) | When the trigger fires, raise that one item to "in Phase 4" and implement; do not gate Phase 3 close on speculative future need. |
| User considers GC leak unacceptable | Documented; can be lifted at any time by a Phase 4 ADR. The infrastructure (ADRs 0184-0186) is in place; lifting safety mode is the remaining work. |
| Phase 4 never starts (no user demand) | Acceptable — the deferrals are honest; nothing in Phase 3 needs them. |

## Future work

- ADR 0191 — Rust-Lua Bridge entry / minimum cut.
- ADR 0192 — Embedded register-ops entry decision.
- Phase 4 entry-criteria ADR (TBD) — written when the first Phase 4 item is scheduled.

## References

- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 closure criteria (referenced for the in-scope items now resolved).
- [ADR 0145](0145-gc-strategy.md) — GC strategy (Phase 2 = leak; Phase 3 = mark-and-sweep — the v1 safety-mode partial implementation).
- [ADR 0153](0153-pcall-error-strategy.md) — pcall / error strategy.
- [ADR 0154](0154-env-true-globals-strategy.md) — `_ENV` strategy.
- [ADR 0155](0155-string-patterns-strategy.md) — string patterns strategy.
- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 entry; this ADR satisfies its §3.
