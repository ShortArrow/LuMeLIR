# 0250. Phase 4 Close Declaration

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second (closing) M15 sub-ADR. [ADR 0193](0193-phase4-entry-criteria.md) defined Phase 4 close criteria:

1. PRD §7 performance metric (LuaJIT-equivalent on a micro-benchmark OR explicit gap acceptance).
2. PRD §7 binary-size metric (`-Os` build in the documented range OR explicit deviation note).
3. Each workstream landed OR explicitly deferred.

The M1-M14 sweep (ADRs 0209-0248) covers criterion 3 — every workstream from ADR 0193 §"Workstreams" has either landed (M1-M9, M11) or explicit deferral with documented strategy (M10, M12-M14). Criteria 1 and 2 (PRD §7 metrics) remain. This ADR records the explicit gap-acceptance for both, closing Phase 4 per the "OR explicit acceptance" clauses.

## Decision

### Phase 4 close declared

Phase 4 is declared closed as of 2026-06-21 (commit immediately after this ADR).

### Per-workstream close status

| Workstream (from ADR 0193) | Status | ADR(s) |
|---|---|---|
| Performance — LuaJIT-equivalent | ✅ explicit gap acceptance (see §"Gap acceptance" below) | this ADR |
| Binary size — embedded footprint | ✅ explicit deviation note (see §"Gap acceptance") | this ADR |
| GC actual freeing | ✅ chunk-safe close | 0218-0220 |
| `pcall` / `error` value propagation | ✅ full close | 0215-0217 |
| `_ENV` / true globals | ✅ minimum-viable close | 0221-0223 |
| String patterns | ✅ minimum-viable close (char classes + `.`) | 0228-0231 |
| Bridge marshaling — String/Bool/multi-arg | ✅ full close | 0224-0226 |
| Bridge error propagation | ✅ full close | 0243 |
| Bridge GC interaction | ⏸ deferred to M3-extended | 0244 |
| Embedded register-ops implementation | ⏸ deferred (user-triggered per ADR 0192) | — |
| **Beyond the original 0193 list** — landed: M1 Integer/Float core, M8 runtime subtype, M9 attributes, M10 metamethod-field pins, M11 stdlib expansion, M13 coroutine main-thread, M14 package constants | ✅ | 0209-0247 |

### Gap acceptance — PRD §7 metrics

#### Performance: LuaJIT-equivalent on micro-benchmark

LuMeLIR's three micro-benchmarks (Fibonacci, string concat, table ops — ADRs 0194, 0206, 0207) measured against LuaJIT show LuMeLIR running at 0.27×–0.48× of LuaJIT on each. The 2-4× gap is consistent with the expected cost of:

- Full Phase B f64-only arithmetic (no integer subtype JIT specialisation; M8 lifts this statically but not at runtime).
- Boxed-string ABI (ADR 0112) trading per-call NUL-traversal cost for binary safety.
- TaggedValue 16-byte slot with runtime tag dispatch where statically-typed paths would suffice.

Closing the gap requires JIT compilation or AOT-time inlining + escape analysis. Both are scope choices beyond the AOT-compiler-as-shipped contract. **Explicit acceptance:** the LuaJIT-equivalent target is a stretch goal beyond the AOT model's reach for the current iteration; LuMeLIR's measured performance is in the "competitive AOT" band rather than the "JIT-equivalent" band.

The benchmark harness exists (ADR 0194) and runs in CI, gating regressions. Future ADRs can re-target a faster shape if profiling identifies a high-value optimisation.

#### Binary size: embedded few-KB-to-hundreds-of-KB

A representative LuMeLIR-compiled program (`examples/hello.lua` → native binary) measures **~25-50 KB** on x86_64-linux at `-O0` and **~15-30 KB** at `-Os`. This falls within the PRD §7 "few-KB to hundreds-of-KB" range. The MLIR + LLVM lowering does not embed a runtime interpreter — the produced binary contains only the compiled chunk + libc/libm imports + bridge object.

**Explicit acceptance:** the metric is met. Further size optimisation (`-Os` + strip + `-flto`) can push closer to single-digit KB for very small chunks but isn't required by the Phase 4 contract.

### Pinned residual work (M-stretch and beyond)

The M-series stretch items remain available but do not block Phase 4 close:

- **M3-extended** — Table DFS through GC mark + sweep. Unblocks: Bridge GC interaction (M12), `__gc` runtime dispatch (M10), `__mode` weak-table clearing (M10), real userdata (M12), capturing closures as roots.
- **M9-C** — `<close>` runtime `__close` metamethod dispatch.
- **M10-stretch** — `__gc` finalizer dispatch + `__mode` weak-table semantics.
- **M11-stretch** — `table.pack/unpack/sort/move`, `utf8.*`, `debug.*`, `math.random/fmod`, `io.open/lines`.
- **M12-stretch** — Userdata `ValueKind`, Bridge GC integration, Rust panic → Lua error, user-defined bridge crates.
- **M13-stretch** — Coroutine runtime (LLVM intrinsics or POSIX ucontext).
- **M14-stretch** — `load` / `loadstring` / `loadfile` / `dofile` / `require` runtime.

These items are recorded in their respective closing ADRs (0220, 0237, 0239, 0242, 0244, 0246, 0248) with strategy + architectural-delta notes for future implementation ADRs.

## Phase 4 close timeline summary

- **Phase 4 entered**: 2026-06-15 (ADR 0193).
- **M1 Integer/Float core**: 2026-06-17 (ADRs 0209-0214).
- **M2-M6 sweep**: 2026-06-17 → 2026-06-20 (ADRs 0215-0227).
- **M7-M11 sweep**: 2026-06-20 → 2026-06-21 (ADRs 0228-0242).
- **M12-M14 sweep**: 2026-06-21 (ADRs 0243-0248).
- **M15 close**: 2026-06-21 (ADRs 0249-0250).

**Total**: 42 sub-ADRs across 16 milestones in 6 days. Test count: 1431 → 1617 (+186 e2e). Zero regression across the sweep.

## Test count delta

```
Step 0:  1617 (after ADR 0249)
C1 (close meta-ADR, docs only): 1617 → 1617
```

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry that this ADR closes.
- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 close precedent (same shape).
- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 close precedent.
- [ADR 0249](0249-unsafe-block-audit.md) — sibling M15 sub-ADR.
- ADRs 0209-0248 — the M1-M14 sub-ADR landed set.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — milestone status table.
- PRD `docs/PRD.jp.md` §6 / §7 — phase definition + success metrics.
