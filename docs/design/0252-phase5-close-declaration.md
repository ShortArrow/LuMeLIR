# 0252. Phase 5 Close Declaration

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M16 sub-ADR. [ADR 0195](0195-phase5-entry-criteria.md) defined Phase 5 close criteria:

1. Coroutines runtime + library — implemented OR explicit deferral.
2. `load` family + module system — implemented OR explicit deferral.
3. Production hardening — each sub-item (CI matrix / release artifacts / security review / unsafe audit) has implementation OR explicit deferral.

The M13-M14-M15-M16 sweep covers all three criteria. This ADR records each close status and declares Phase 5 closed with explicit deferral of the substantial implementation items.

## Decision

### Phase 5 close declared

Phase 5 is declared closed as of 2026-06-21.

### Per-workstream close status

| Workstream (from ADR 0195) | Status | ADR(s) |
|---|---|---|
| Coroutines runtime | ⏸ deferred via documented strategy | 0246 |
| coroutine library (`create` / `resume` / `yield` / `wrap` / `status` / `running` / `isyieldable`) | ✅ `isyieldable` + `running` shipped (main-thread spec-correct); remaining 5 functions deferred alongside runtime | 0245 + 0246 |
| `load` / `loadfile` / `dofile` | ⏸ deferred via documented strategy (embedded-pipeline vs. external-bytecode-loader vs. pre-baked AOT vs. userdata-handle) | 0248 |
| `require` / `package.*` | ✅ `package.config` / `path` / `cpath` shipped (HIR pre-pass synthesis); `require` runtime deferred with `load` | 0247 + 0248 |
| Production hardening — CI matrix | ✅ x86_64-linux gate active; multi-target matrix queued with documented order | 0251 |
| Production hardening — release artifacts | ⏸ explicit deferral (see §"Release artifacts deferral" below) | this ADR |
| Production hardening — security review | ✅ `cargo audit` step added to CI; unsafe-block audit shipped (ADR 0249) | 0249 + 0251 |
| Production hardening — `unsafe`-block audit | ✅ full audit | 0249 |

### Release artifacts deferral

GitHub Releases with signed binaries + automated changelog is a real production-hardening item, but it requires:

- A versioning scheme decision (semver? `v0.x` pre-1.0 nightly?).
- A release-trigger workflow (push tag → build + sign + upload).
- Per-target binaries (depends on the multi-target CI matrix being active).
- A signing-key management policy (cosign? GPG? hardware-token-backed?).

The first user-visible release requires all four. Until concrete user demand surfaces a target platform that needs the release artifact pipeline, this ADR records the deferral with explicit acceptance: producing release artifacts is a one-time setup investment best made when the first release demand arrives. The CI gates (clippy + test + audit + fmt) already certify each commit; users building from source get the same artifact as a release binary would.

### Phase 5 — what shipped, what deferred

**Shipped:**
- ADR 0245: `coroutine.isyieldable` + `coroutine.running` main-thread.
- ADR 0246: coroutine runtime strategy decision + deferral.
- ADR 0247: `package.config` / `path` / `cpath` synthesis.
- ADR 0248: load/require runtime strategy + deferral.
- ADR 0249: unsafe-block audit (31 sites enumerated).
- ADR 0250: Phase 4 close declaration.
- ADR 0251: CI matrix + `cargo audit` step.
- This ADR (0252): Phase 5 close declaration.

**Deferred with strategy + architectural-delta notes:**
- Coroutine runtime (LLVM intrinsics or POSIX ucontext) — ADR 0246.
- `load` / `require` runtime (embedded-pipeline preferred) — ADR 0248.
- Multi-target CI matrix activation (aarch64-linux, arm64-darwin, x86_64-windows) — ADR 0251.
- Release artifact pipeline — this ADR.

### Phase 5 timeline

- **Phase 5 entered**: 2026-06-15 (ADR 0195).
- **M13 close**: 2026-06-21 (ADRs 0245-0246).
- **M14 close**: 2026-06-21 (ADRs 0247-0248).
- **M15 close + Phase 4 close**: 2026-06-21 (ADRs 0249-0250).
- **M16 close + Phase 5 close**: 2026-06-21 (ADRs 0251-0252).

Phase 4 and Phase 5 both closed on the same day after a 6-day sub-ADR sweep that landed 44 minor ADRs (0209-0252) — a meta-pattern worth noting for future planning: per-milestone minimum-viable closes can compress what was planned as multi-month work into compressed bursts when the underlying machinery is in place and the residual implementation items can defer cleanly.

## Project status after Phase 5 close

LuMeLIR is now in a **stable Lua 5.4 preferred-subset AOT compiler** state with documented residual work catalogued across the M-stretch deferrals. Remaining work (collectively the "real Lua 5.4 100% conformance" target):

- M3-extended: Tables in GC mark/sweep (unblocks ADRs 0220, 0238, 0239, 0244 runtime semantics).
- M9-C: `__close` runtime hook.
- M10-stretch: `__gc` finalizer dispatch, `__mode` weak-table clearing.
- M11-stretch: stdlib expansion (`table.pack/unpack/sort`, `utf8.*`, `debug.*`, `math.random/fmod`, `io.open/lines`).
- M12-stretch: userdata `ValueKind`, Bridge GC integration.
- M13-stretch: coroutine runtime (largest item).
- M14-stretch: `load` / `require` runtime (second largest item).
- Performance: JIT or AOT-time inlining + escape analysis to close the LuaJIT performance gap.

Each item has its closing ADR documenting strategy + architectural deltas; future implementation ADRs can pick any single item up without rederiving the scope.

## M17 status

[Roadmap status](../notes/lua54-full-conformance-path-2026-06-20.md) §M17 = "Lua 5.4 preferred-subset 網羅宣言" — a 0.5-session meta-ADR. After Phase 5 close, M17 can land whenever the project decides to make the preferred-subset declaration formal. The current state (this ADR) is the substance of that declaration; M17 would be a one-paragraph summary committing to the contract.

## Test count delta

```
Step 0:  1617 (after ADR 0251)
C1 (close meta-ADR, docs only): 1617 → 1617
```

## References

- [ADR 0195](0195-phase5-entry-criteria.md) — Phase 5 entry that this ADR closes.
- [ADR 0250](0250-phase4-close-declaration.md) — Phase 4 close precedent (same shape).
- ADRs 0245-0251 — the Phase 5 sub-ADR landed set.
- [Roadmap status 2026-06-20](../notes/roadmap-status-2026-06-20.md) — milestone status table.
- [Lua 5.4 full conformance path](../notes/lua54-full-conformance-path-2026-06-20.md) — M17 reference.
- PRD `docs/PRD.jp.md` — project scope + success metrics.
