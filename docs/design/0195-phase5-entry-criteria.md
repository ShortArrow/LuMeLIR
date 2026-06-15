# 0195. Phase 5 Entry Criteria + Scope Freeze (Lua 5.4 Full Conformance + Production Hardening)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0193](0193-phase4-entry-criteria.md) opened Phase 4 (post-PRD completeness + optimisation). The [Lua 5.4 conformance roadmap memo](../notes/lua54-conformance-roadmap.md) (2026-06-15) enumerated the remaining gap to full spec conformance and identified Phase 5 as the natural home for **Coroutines + production hardening** in §Recommended phase decomposition. ADR 0193 §Future work also forward-referenced Phase 5 as "optional, only if Phase 4 surfaces a need for it".

Phase 4 is already substantial (per the roadmap: 50-65 sessions to finish §4a + §4b + §4c). Phase 5 captures the parts of Lua 5.4 conformance that are too disruptive to slot inside Phase 4 alongside performance and stdlib work:

1. **Coroutines** — runtime mountain (continuation stack, yield/resume, GC root expansion). Defer to its own phase per the roadmap.
2. **`load` / `loadfile` / `dofile`** — self-hosting-adjacent (runtime parser + codegen invocation). Most production-Lua programs don't `load`, so deferring outside Phase 4 keeps the more-impactful 4a/4b work unblocked.
3. **Production hardening** — CI matrix, multi-target QA, release artifacts, security review of the runtime allocator and the Bridge. None of these fit Phase 4's "implement the spec" framing.

This ADR pins those items as Phase 5 scope.

## Decision

### Phase 5 = Coroutines + Runtime Evaluation + Production Hardening

Phase 5 closes when each workstream below has implementation OR explicit deferral to a future phase recorded. Mirror of ADR 0133 / ADR 0189 / ADR 0193 close pattern.

### Workstreams

| Workstream | Source | Triggers / blockers |
|---|---|---|
| **Coroutines runtime** (continuation stack + yield/resume + GC root expansion) | Lua 5.4 §2.6 + roadmap | The largest single Phase 5 item. Needs ADR 0190 GC freeing landed (so the coroutine root set has somewhere to register). Multi-ADR sub-sweep expected. |
| **coroutine library** (`create` / `resume` / `yield` / `wrap` / `status` / `running` / `isyieldable`) | Lua 5.4 §6.2 | Lands as a thin layer over the coroutine runtime. |
| **`load` / `loadfile` / `dofile`** (runtime evaluation) | Lua 5.4 §6.1 | Self-hosting-adjacent. Requires linking `lumelir::parser` + `lumelir::hir` + `lumelir::codegen` into the compiled runtime — or a runtime-side mini-interpreter. Architectural decision. |
| **`require` / `package.*`** (module system) | Lua 5.4 §6.3 | Depends on `load` and on a filesystem-search policy decision. |
| **Production hardening — CI matrix** | New | Linux x86_64 / Linux aarch64 / macOS / Windows. Run the full 1431+ test corpus on each. |
| **Production hardening — release artifacts** | New | Versioned releases, signed binaries, changelog automation, GitHub release workflow. |
| **Production hardening — security review** | New | Audit allocator (`emit_gc_alloc` chain), Bridge surface, runtime libc usage. Document threat model. |
| **Production hardening — `unsafe`-block audit** | New | Currently allowed `unsafe`-free surface (per ADR-precedent); enforce via `#![deny(unsafe_code)]` at crate level + document any unavoidable exceptions. |

### Phase 5 close criteria

Phase 5 closes when:

1. **Coroutines runtime + library** — implemented OR explicit deferral to a hypothetical Phase 6 with rationale.
2. **`load` family + module system** — implemented OR explicit deferral.
3. **Production hardening** — each sub-item (CI matrix / release artifacts / security review / unsafe audit) has implementation OR explicit deferral.

When all three hold, Phase 5 close is declared in this doc via the status-line update precedent (ADR 0133 / 0189 / 0193).

### Phase 5 is realistically multi-month

Per the Lua 5.4 conformance roadmap:

- Coroutines: 3-5 ADRs, 4-6 sessions
- coroutine library: 1-2 ADRs, 1-2 sessions
- `load` family: 2-3 ADRs, 4-6 sessions (large)
- module system: 3-5 ADRs, 3-5 sessions
- Production hardening: 4-6 ADRs, 4-8 sessions

**Aggregate**: 13-21 ADRs, 16-27 sessions. Roughly **1-2 months** of focused work *after* Phase 4 lands.

A goal of "Phase 5 完了 in one session" is not achievable; the goal is a directional anchor, not a session-bound deliverable. The hook (if set) is expected to remain active across many sessions until the close criteria are met.

### Ordering within Phase 5

Recommended sequencing:

1. **Coroutines runtime first** — the largest dependency, blocks coroutine library. Begin with the design ADR pinning the continuation-stack ABI.
2. **coroutine library** — thin layer; lands quickly once runtime exists.
3. **Production hardening track in parallel** — CI matrix and unsafe audit are independent of coroutines; can run alongside.
4. **`load` family** — large, can wait until after coroutines settle so the architectural debate has fewer moving parts.
5. **Module system** — depends on `load`; lands last in the language-feature track.
6. **Security review + release artifacts** — capstone, after all features land.

### Phase 4 prerequisite

Phase 5 explicitly **depends on Phase 4 closing**. Specifically:

- Coroutines need ADR 0190 GC actual freeing (workstream in Phase 4) before the continuation-stack root set can be tracked safely.
- The Lua 5.4 conformance roadmap's Phase 4a (Integer/Float subtype, pcall/error, GC freeing, etc.) must land first; running Phase 5 against an incomplete Phase 4 language base is wasted work.

If a Phase 5 item is started before Phase 4 closes, that ADR carries an explicit cross-phase dependency note in its §Risks.

## Scope (literal)

- ✅ Phase 5 = Coroutines + Runtime Evaluation (`load`/`require`) + Production Hardening.
- ✅ Workstream table above is the exhaustive list at ADR-write time. New items surface via per-item ADRs that update this table.
- ✅ Ordering is recommended, not mandated.
- ✅ Phase 5 depends on Phase 4 closing — explicitly noted.
- ❌ Implement any workstream now. This ADR is decision-only.
- ❌ Commit to a Phase 6. Future phases are TBD-on-need.
- ❌ Define the C API (`§4` of the Lua 5.4 reference manual). LuMeLIR is AOT-only; embedding LuMeLIR in a C host is a separate project direction, not Phase 5 scope.

## Alternatives considered

- **Skip Phase 5; fold coroutines + `load` + hardening into Phase 4.** Rejected — Phase 4 is already 50-65 sessions per the roadmap; adding 16-27 more inflates the close window beyond any reasonable plannable horizon. Separating gives both phases their own close criteria.
- **Defer Phase 5 declaration until Phase 4 actually closes.** Rejected — knowing Phase 5's scope now lets future ADRs reference it (e.g. "this work belongs in Phase 5 per ADR 0195") instead of leaving the home undecided.
- **Bundle production hardening as its own Phase 6.** Rejected — production hardening is small relative to coroutines + `load`; doesn't warrant its own phase. Folding into Phase 5 lets the project ship as "Phase 5 complete = production-ready Lua 5.4-conformant compiler".
- **Skip `load` / `require` entirely.** Rejected — many real Lua programs use `require` for modules. Static-only Lua is a defensible niche but disqualifies "Lua 5.4 conformant" claim.

## Consequences

**Positive**
- Phase 5 has an entry point + workstream list; future work has a home.
- "Lua 5.4 full conformance + production-ready" is a concrete project milestone with checkable criteria.
- ADR 0193 §Future work bullet "Phase 5 entry-criteria ADR (TBD, only if Phase 4 surfaces a need)" is satisfied; we surfaced the need via the conformance roadmap.

**Negative**
- One more meta-ADR before concrete work. Cost: 1 commit; benefit: scope discipline at scale.
- Phase 5 close is realistically months away; the goal of "Phase 5 完了" is an anchor across many sessions, not a session-bound target.

**Locked in until superseded**
- "Phase 5 = Coroutines + Runtime Eval + Hardening" is the contract.
- "Phase 5 depends on Phase 4 closing" is the contract.
- Adding a new workstream requires updating this table in the same PR.

## Documentation updates

- [x] §8 — adds 0195.
- [x] ADR 0193 §Future work — Phase 5 ADR now exists; cross-reference here.
- [x] Lua 5.4 conformance roadmap — Phase 5 home pinned to coroutines + `load` + hardening.

## Test count delta

```
Step 0: 1431 (after 6f71a80)
C1 (this doc): 1431 → 1431 (decision-only, no test)
```

## Critical files

- `docs/design/0195-phase5-entry-criteria.md` (this doc).
- `docs/design/README.md` index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Phase 5 sprawl (each workstream balloons mid-implementation) | Per-item ADRs enforce scope literals per the ADR 0133 / 0189 / 0193 pattern. |
| Coroutines runtime design surprises (continuation-stack ABI conflicts with closure cell layout per ADR 0083) | First Phase 5 implementation ADR is the coroutine runtime design memo; pre-flight review (analog to `gc-0159-0162-preflight-review.md`) catches surprises before commits. |
| `load` family architecturally too large | If `load` becomes a 10+-ADR arc, split into Phase 6 instead of dragging Phase 5 close. |
| Phase 4 never closes; Phase 5 starts anyway | Per-ADR cross-phase dependency note flags the order violation. Decision to proceed lies with the author. |
| Production hardening surfaces architectural rework | Accept it; that's the point of the hardening track. Each surface gets its own follow-up ADR. |

## Future work

- ADR (TBD, first Phase 5 implementation) — Coroutine runtime design (continuation-stack ABI + yield/resume protocol + GC root expansion).
- Per-workstream ADRs in §Ordering order.
- Phase 6 entry-criteria ADR (TBD, only if Phase 5 surfaces a workstream that doesn't fit).

## References

- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 close meta-ADR; precedent.
- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 entry meta-ADR; precedent.
- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry meta-ADR; direct precedent.
- [Lua 5.4 Conformance Roadmap](../notes/lua54-conformance-roadmap.md) — surfaced the need for Phase 5.
- [Lua 5.4 Reference Manual §2.6](https://www.lua.org/manual/5.4/manual.html#2.6) — coroutines spec.
- [Lua 5.4 Reference Manual §6.1](https://www.lua.org/manual/5.4/manual.html#6.1) — `load` family.
- [Lua 5.4 Reference Manual §6.3](https://www.lua.org/manual/5.4/manual.html#6.3) — module system.
