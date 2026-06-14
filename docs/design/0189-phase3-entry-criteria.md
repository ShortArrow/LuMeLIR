# 0189. Phase 3 Entry Criteria + Scope Freeze (Rust-Lua Bridge First)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0133](0133-phase2-completion-criteria.md) closed Phase 2 (formally declared 2026-06-09 in commit `60932bf`). PRD §6 names Phase 3 as **Domain Specific Features** — specifically two themes:

1. **Rust-Lua Bridge** — Rust 関数を MLIR レベルでインライン化して Lua から呼び出す機能.
2. **組み込み用レジスタ操作方言の統合** — per-target register-ops dialect.

The PRD memo (§ "メモ") explicitly identifies **the Rust-Lua Bridge as the kimo (キモ)**: "普段書いている Rust プロジェクトに、オーバーヘッドほぼゼロで Lua の柔軟性を持ち込めるようになる". The embedded-dialect theme is parallel but lower-leverage for the project's stated goal.

Phase 2 also left several **language-completion decisions** as "Phase 3 trigger (decision-only)":

- [ADR 0153](0153-pcall-error-strategy.md) — pcall / error value propagation
- [ADR 0154](0154-env-true-globals-strategy.md) — `_ENV` / true globals
- [ADR 0155](0155-string-patterns-strategy.md) — `string.find` / `match` / `gmatch` / `gsub`
- [ADRs 0156-0162](0156-gc-architecture-v1.md) — GC mark-and-sweep architecture (partially implemented as ADRs 0184-0186 in v1 safety mode; actual freeing pending ADR 0189+ stack walk)

These are language features Phase 2 explicitly deferred because they touch enough of HIR / codegen / runtime that landing them inside Phase 2 would have blocked the close. They are PRD-Phase-2 in spirit (core semantics) but were sequenced post-Phase-2 in practice. This ADR labels them **Phase 2 epilogue** and explicitly distinguishes them from **PRD Phase 3** work.

Without an entry-criteria meta-ADR, the next concrete ADR (Rust-Lua Bridge minimum cut) risks sprawling into "design the whole bridge protocol". Same scope-creep risk Codex flagged before ADR 0133.

## Decision

### Phase 3 = PRD Domain Specific Features

Phase 3 closes when each PRD-named workstream below has an Architecture Decision recorded; **implementation may stagger as sub-phase work** without blocking the close (mirror of ADR 0133's pattern).

**In Phase 3** (this set must land before declaring Phase 3 done):

- **Rust-Lua Bridge — minimum cut**: ADR for the smallest principled exposure of Rust functions to Lua. Sub-pieces (ABI, marshaling, error propagation, GC interaction) get their own ADRs underneath. **Triggered first** per the PRD memo's "kimo" claim.
- **Embedded register-ops dialect — entry decision**: ADR pinning the first target (likely Cortex-M or RISC-V), the volatile-store semantics, and the integration point with `llvm.func`. Implementation deferred unless a concrete user emerges.

### Phase 2 epilogue (not Phase 3 — runs in parallel)

These are PRD Phase 2-spirit but post-close in practice:

| Workstream | Status | Next ADR |
|---|---|---|
| GC mark + sweep + stack walk (real freeing) | v1 safety-mode landed (ADRs 0184-0186); stack walk + DFS lift to land via ADR 0190+ | 0190 |
| `pcall` / `error` value propagation | Designed (ADR 0153); not implemented | follow-up |
| `_ENV` / true globals | Designed (ADR 0154); not implemented | follow-up |
| `string` patterns (`find` / `match` / `gmatch` / `gsub`) | Designed (ADR 0155); not implemented | follow-up |

The epilogue work and Phase 3 work run **concurrently**. Phase 3 does not block on epilogue completion; epilogue items land as their priority surfaces (e.g. `pcall` becomes hard-required when the Bridge needs to propagate Rust panics into Lua errors).

### Ordering for Phase 3 itself

1. **ADR 0190 = GC mark-phase DFS + stack walk** (epilogue, lands the first real `collectgarbage()` freeing). Required precondition for the Bridge so Rust-owned Lua values participate in GC correctly.
2. **ADR 0191 = Rust-Lua Bridge entry / minimum cut**. Scope ceiling decision: which Rust signatures are exposed, what marshaling rules apply, what's deferred. Mirror of ADR 0134's "smallest principled cut" approach.
3. **ADR 0192+ = Bridge sub-pieces** as needed (error propagation when `pcall` epilogue lands; GC interaction when stack walk lands; type marshaling when first non-Number signature is exercised).
4. **ADR 0xxx = Embedded register-ops entry** (parallel, lower priority; may slip past Phase 3 close declaration if no user emerges).

### Phase 3 close criteria

Phase 3 closes when:

1. Rust-Lua Bridge entry ADR exists AND a minimum-viable Rust function (e.g. `fn add(a: f64, b: f64) -> f64`) is callable from Lua and verified end-to-end.
2. Embedded register-ops entry ADR exists (implementation optional unless user emerges).
3. The 4 Phase 2 epilogue items each have an "implemented" or explicit "deferred to Phase 4" decision recorded.

PRD success metrics (LuaJIT-equivalent performance, embedded binary size) are **not Phase 3 close requirements** — they are project-wide targets that surface optimisation passes and codegen tightening, which sit in Phase 4+ territory.

## Alternatives considered

- **Skip the meta-ADR; write the Bridge entry ADR directly.** Rejected — Codex review on ADR 0133 explicitly flagged scope creep without a pre-stated boundary; same risk here. The Bridge has many natural sub-pieces (ABI / marshaling / error / GC / async?) that need explicit deferral.
- **Treat the Phase 2 epilogue items as Phase 3.** Rejected — they are language completions defined by PRD Phase 2 ("core semantics"); calling them Phase 3 muddies the close-criteria definition both for Phase 2 (already closed) and Phase 3.
- **Start with the embedded dialect instead of the Bridge.** Rejected — PRD memo explicitly identifies the Bridge as the project kimo; ordering should follow the stated value claim.
- **Block Phase 3 on epilogue completion.** Rejected — the epilogue items are independent of the Bridge for most cases; sequencing them would delay Phase 3 entry by multiple sessions. The Bridge can start with a Number-only signature and grow alongside epilogue landings.

## Consequences

**Positive**
- Phase 3 has a concrete entry point and ordered workstreams.
- The Phase 2 epilogue / Phase 3 distinction is explicit; future ADRs do not get mislabelled.
- Bridge minimum cut has a scope ceiling before its design starts.
- Parallel epilogue + Phase 3 work avoids gating delays.

**Negative**
- One more meta-ADR ahead of the concrete work. Cost: 1 commit; benefit: scope discipline.
- Phase 4+ remains unscoped (performance / binary size). Acceptable — those are evaluation tasks, not feature tasks.

**Locked in until superseded**
- "Phase 3 = PRD Domain Specific Features (Bridge + Embedded dialect)" is the contract.
- "Phase 2 epilogue items run in parallel to Phase 3" is the contract.
- Adding a new Phase 3 workstream requires updating this table in the same PR.

## Documentation updates

- [x] §8 — adds 0189.
- [x] ADR 0133 cross-reference — Phase 2 close declaration; this ADR picks up from there.
- [x] Sweep 0182-0188 retrospective — "Next chokepoint candidates" §; ADR 0189 supersedes the freeform recommendation with a structured entry.

## Test count delta

```
Step 0: 1427 (after cfef4ee)
C1 (this doc): 1427 → 1427 (decision-only, no test)
```

## Critical files

- `docs/design/0189-phase3-entry-criteria.md` (this doc).
- `docs/design/README.md` index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Bridge sub-pieces sprawl (ABI ADR + marshaling ADR + error ADR + GC ADR + ...) | Each sub-piece gets its own scope ceiling per ADR 0133 pattern; ADR 0191 is the smallest principled cut, not the whole protocol. |
| Epilogue items block in practice when Bridge needs them | Documented dependency: ADR 0190 (GC stack walk) is sequenced first; pcall propagation is sequenced when Bridge first needs to surface a Rust error. |
| Embedded-dialect work never starts (no user) | Acceptable per PRD priority. Phase 3 close criteria allow embedded ADR with implementation deferred. |
| PRD success metrics treated as Phase 3 requirements | Explicitly excluded from close criteria; documented as Phase 4+ territory. |
| ADR 0189 numbering collides with planned ADR 0190 mark-DFS | This ADR is 0189, GC mark-DFS becomes 0190 per §Ordering. |

## Future work

- ADR 0190 — GC mark-phase DFS + stack walk implementation (epilogue, sequenced first per §Ordering).
- ADR 0191 — Rust-Lua Bridge entry / minimum cut.
- ADR 0192+ — Bridge sub-pieces as triggered.
- ADR (TBD) — Embedded register-ops entry decision.
- Phase 4 (TBD) — optimisation passes for LuaJIT-equivalent performance; binary-size tightening for embedded.

## References

- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 closure criteria; direct precedent for this meta-ADR's shape.
- [ADR 0145](0145-gc-strategy.md) — GC strategy (Phase 2 = leak; Phase 3 = mark-and-sweep).
- [ADR 0153](0153-pcall-error-strategy.md) — pcall / error (epilogue).
- [ADR 0154](0154-env-true-globals-strategy.md) — `_ENV` (epilogue).
- [ADR 0155](0155-string-patterns-strategy.md) — string patterns (epilogue).
- [Sweep 0182-0188 retrospective](../notes/sweep-0182-0188-retrospective.md) — "Next chokepoint candidates" § superseded by §Ordering here.
- PRD `docs/PRD.jp.md` §6 (Phase 3) and § "メモ" (Bridge as kimo).
