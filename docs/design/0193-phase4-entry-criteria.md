# 0193. Phase 4 Entry Criteria + Scope Freeze (Post-PRD Completeness + Optimisation)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0189](0189-phase3-entry-criteria.md) closed Phase 3 (formally declared 2026-06-15 in commit `56eca3c`). PRD §6 lists Phases 1-3 only — Phase 4 is **project-internal**, not PRD-defined. The PRD memo ("メモ") and §7 success metrics still gate full project completion, so Phase 4 has scope but no PRD-anchored end date.

Phase 4 inherits four buckets of work surfaced by prior ADRs:

- **PRD success metrics (§7)** — LuaJIT-equivalent performance on simple arithmetic; embedded binary size in the few-KB-to-hundreds-of-KB range.
- **Phase 2 epilogue items** ([ADR 0190](0190-phase2-epilogue-close-decisions.md)) — GC actual freeing, `pcall` / `error`, `_ENV` / true globals, string patterns.
- **Bridge sub-pieces** ([ADR 0191](0191-rust-lua-bridge-mvp.md) §Future work) — type marshaling, error propagation, GC interaction.
- **Embedded register-ops implementation** ([ADR 0192](0192-embedded-register-ops-entry.md)) — target selection + codegen lowering, user-triggered.

Without an entry-criteria meta-ADR, the next concrete ADR risks the same scope sprawl Codex flagged before ADRs 0133 and 0189. This ADR pins the buckets and ordering rationale.

## Decision

### Phase 4 = Post-PRD Completeness + Optimisation

Phase 4 closes when the PRD success metrics are demonstrably met AND each epilogue / sub-piece workstream has either implementation or explicit deferral recorded. Mirror of ADR 0133 / ADR 0189 close pattern.

### Workstreams (decision-only entry; per-item ADRs follow)

| Workstream | Source | Trigger / blocker |
|---|---|---|
| Performance — LuaJIT-equivalent on simple arithmetic | PRD §7 | Needs benchmark harness + optimisation passes (inlining, escape analysis, constant fold) |
| Binary size — few-KB to hundreds-of-KB embedded footprint | PRD §7 | Needs `-Os` tuning, runtime trimming, tagged-ABI minimisation |
| GC actual freeing (stack walk + DFS lift) | ADR 0190 | ADRs 0184-0186 ship machinery; stack walk per ADR 0160 design is the remaining piece |
| `pcall` / `error` value propagation | ADR 0153 | Triggered when first Bridge sub-piece needs Rust panic → Lua error surfacing |
| `_ENV` / true globals | ADR 0154 | Triggered when first Bridge consumer needs cross-boundary mutated globals |
| String patterns (`find` / `match` / `gmatch` / `gsub`) | ADR 0155 | Self-contained; lands on direct user demand |
| Bridge marshaling — String / Table / Bool / TaggedValue | ADR 0191 §Future | Triggered by first non-Number Bridge signature |
| Bridge error propagation | ADR 0191 §Future | Paired with `pcall` epilogue |
| Bridge GC interaction | ADR 0191 §Future | Paired with GC actual freeing |
| Embedded register-ops implementation | ADR 0192 | User-triggered; picks target + LLVM lowering |

### Ordering rationale

No single forced ordering. The dependency edges that exist:

- Bridge error propagation **needs** `pcall` epilogue.
- Bridge GC interaction **needs** GC actual freeing.
- Performance + binary-size work is largely orthogonal to the others — can run in parallel.

Sequencing recommendation when work resumes:

1. **Benchmark harness first** — without a measurement floor, performance work is speculative. One ADR to define the benchmark suite (e.g. Fibonacci, prime sieve, string concat) and the LuaJIT comparison protocol.
2. **GC actual freeing** — unblocks Bridge GC interaction and removes the v1 leak observable. Largest known piece; multi-session.
3. **Bridge sub-pieces in trigger order** — marshaling first (most common need), then error propagation (paired with `pcall`), then GC interaction (paired with GC freeing).
4. **Language completions** (`pcall` / `_ENV` / string patterns) — interleave with Bridge sub-pieces or stand-alone.
5. **Optimisation passes** — once benchmarks reveal hot paths.
6. **Binary-size work** — last; needs the full feature surface stable to know what to trim.
7. **Embedded register-ops implementation** — user-triggered; may slip past Phase 4 close declaration.

### Phase 4 close criteria

Phase 4 closes when:

1. **PRD §7 performance metric** — a published benchmark report shows LuaJIT-equivalent performance on at least one micro-benchmark from the harness (criterion 1 from Phase 4 §Ordering). Either "achieved" or "explicit acceptance of the gap with rationale" satisfies this.
2. **PRD §7 binary-size metric** — a measured `-Os` build of a representative Lua program is in the documented "few-KB to hundreds-of-KB" range, OR an explicit deviation note documents the actual figure and the gap rationale.
3. **Each workstream above** has implementation OR explicit deferral to a future phase recorded.

When all three hold, Phase 4 close is declared in this doc via an update (precedent: ADR 0133 / ADR 0189 status-line update).

## Scope (literal)

- ✅ Phase 4 = post-PRD completeness + optimisation, defined as the bucket of work that closes the PRD §7 metrics and resolves all open workstreams.
- ✅ Workstream table above is the exhaustive list at ADR-write time. New items surface via per-item ADRs that update this table.
- ✅ Ordering is recommended, not mandated. Each ADR may justify a different ordering for its own item.
- ❌ Implement any workstream now. This ADR is decision-only.
- ❌ Pick a specific benchmark target (e.g. "Fibonacci(30)"). The benchmark-harness ADR (criterion 1) picks the suite.
- ❌ Commit to a Phase 5. Future phases (e.g. "production hardening" / "ecosystem") are not in scope until Phase 4 produces a need for them.

## Alternatives considered

- **Skip the meta-ADR; let each Phase 4 work item start without a roadmap.** Rejected — precedent (ADR 0133 / ADR 0189) shows entry meta-ADRs prevent scope creep. Same discipline applies.
- **Declare Phase 4 unbounded** ("Phase 4 ends when LuMeLIR is deemed done"). Rejected — without close criteria, Phase 4 has no exit. ADR 0133 / ADR 0189 both had close criteria; consistency.
- **Bundle PRD §7 metrics as Phase 5.** Rejected — PRD §7 is the project's stated success measure; landing it in a separate phase pushes the success declaration past Phase 4 unnecessarily.
- **Skip Phase 4 entirely and start direct on individual ADRs.** Rejected — same scope-creep risk Codex flagged twice; no reason to break the pattern.

## Consequences

**Positive**
- Phase 4 has a concrete entry point and an enumerated workstream list.
- PRD §7 success metrics gate Phase 4 close; the project's stated success measure is wired into the close criteria.
- Bridge sub-pieces and epilogue items have a documented home; no open question about which phase they belong to.
- Each workstream can produce its own per-item ADR without re-deriving the phase scope.

**Negative**
- One more meta-ADR ahead of concrete work. Cost: 1 commit; benefit: scope discipline.
- Phase 4 close criteria depend on benchmarking (criterion 1), which has not yet been designed. The benchmark-harness ADR is the first Phase 4 implementation work.

**Locked in until superseded**
- "Phase 4 = post-PRD completeness + optimisation, gated on PRD §7 metrics" is the contract.
- "Each workstream above has implementation OR explicit deferral" is the per-item exit condition.
- Adding a new workstream requires updating this table in the same PR.

## Documentation updates

- [x] §8 — adds 0193.
- [x] ADR 0190 — "Phase 4 entry signal" → satisfied here; cross-reference.

## Test count delta

```
Step 0: 1430 (after 56eca3c)
C1 (this doc): 1430 → 1430 (decision-only, no test)
```

## Critical files

- `docs/design/0193-phase4-entry-criteria.md` (this doc).
- `docs/design/README.md` index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Benchmark harness ADR (criterion 1) gets infinite scope | First Phase 4 ADR enforces its own scope literal per the ADR 0133 / ADR 0189 pattern. |
| Performance work proves LuaJIT-equivalent is unreachable | Close criterion §1 allows "explicit acceptance of the gap with rationale" — honest documentation beats moving the goalpost. |
| Bridge GC interaction blocks on GC freeing which blocks on stack walk | Linear chain documented; sequencing recommendation §2 → §3 honors this. |
| Workstream list incomplete | The table is exhaustive at ADR-write time. New workstreams must update this table per §Locked in. |
| Phase 4 never starts (project ends here) | Acceptable — Phase 3 satisfies PRD §6's explicit scope. Phase 4 is project-internal continuation; closing the doc now does not break anything. |

## Future work

- ADR 0194 — Benchmark harness (Phase 4 criterion 1 prerequisite). First Phase 4 implementation ADR.
- ADR 0195+ — Per-workstream ADRs in trigger order: GC stack walk, Bridge marshaling, etc.
- Phase 5 entry-criteria ADR (TBD, only if Phase 4 surfaces a need).

## References

- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 close meta-ADR; precedent for shape.
- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 entry meta-ADR; direct precedent.
- [ADR 0190](0190-phase2-epilogue-close-decisions.md) — epilogue Phase 4 deferrals; this ADR satisfies its "Phase 4 entry signal" forward reference.
- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP; sub-pieces are Phase 4 workstreams.
- [ADR 0192](0192-embedded-register-ops-entry.md) — Embedded entry; implementation is a Phase 4 workstream.
- PRD `docs/PRD.jp.md` §7 — success metrics gating Phase 4 close.
