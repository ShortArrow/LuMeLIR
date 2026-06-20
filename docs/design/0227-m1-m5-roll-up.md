# 0227. M1-M5 Roll-Up — Phase 4a Language-Level Subset Close

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

[Roadmap revision 2026-06-16](../notes/roadmap-revision-2026-06-16.md) replaced bucket-counting goals with milestone-based session goals M1-M6. M1-M5 have landed:

| Milestone | Sub-ADRs | Tests delta | Status |
|---|---|---|---|
| M1 Integer/Float subtype core | 0209-0214 (6) | 1464 → 1478 | ✅ minimal close |
| M2 pcall / error propagation | 0215-0217 (3) | 1478 → 1485 | ✅ full close |
| M3 GC actual freeing | 0218-0220 (3) | 1485 → 1494 | ✅ full close (chunk-safe) |
| M4 `_ENV` / true globals | 0221-0223 (3) | 1494 → 1506 | ✅ minimum-viable close |
| M5 Bridge marshaling | 0224-0226 (3) | 1506 → 1519 | ✅ full close |

**M6 = this ADR.** Per [`docs/notes/lua54-full-conformance-path-2026-06-20.md`](../notes/lua54-full-conformance-path-2026-06-20.md) §M6, M6 is the meta-ADR declaring Phase 4a's language-level subset closed after M2-M5 land. M1 was extra-roadmap (Integer/Float predates Phase 4a buckets).

Per [ADR 0193](0193-phase4-entry-criteria.md), full Phase 4 close requires PRD §7 performance + binary-size metrics in addition to language workstreams. This ADR is the language-level subset close — analogous to the informal "Phase 4a" name used in [`leftover-roadmap.md`](../notes/leftover-roadmap.md). It does NOT declare Phase 4 closed; PRD §7 work remains.

## Decision

### Phase 4a language-level subset declared closed

The five workstreams landed satisfy [ADR 0193](0193-phase4-entry-criteria.md) §"Workstreams" rows:

- ✅ **GC actual freeing** — M3 (ADRs 0218-0220). Chunk-safe predicate covers String / Function / TaggedValue locals + non-capturing user fns; programs with Tables in chunk slots stay in v1 safety mode.
- ✅ **`pcall` / `error` value propagation** — M2 (ADRs 0215-0217). Single-return Bool + multi-return `(ok, err)` ABI; setjmp/longjmp infrastructure.
- ✅ **`_ENV` / true globals** — M4 (ADRs 0221-0223). `_G` / `_ENV` chunk-level Table via HIR pre-pass; sandbox rebind via standard Lua shadowing.
- ✅ **Bridge marshaling — String / Bool / Multi-arg mixed-kind** — M5 (ADRs 0224-0226). Per-kind ABI shapes (ptr-passthrough for String, i1↔i8 hop for Bool, Number from ADR 0191) compose into multi-arg signatures.
- ✅ **Integer/Float subtype core** — M1 (ADRs 0209-0214). HIR `Integer(i64)` variant; `math.type` / `math.tointeger` / `math.maxinteger` / `math.mininteger` static-shape; constant fold; subtype-preserving print.

### What this ADR does NOT close

Phase 4 (full) remains open per ADR 0193 §"Phase 4 close criteria":

1. PRD §7 performance metric (LuaJIT-equivalent micro-benchmark report or explicit gap acceptance).
2. PRD §7 binary-size metric (`-Os` build measurement in the documented range or gap explanation).
3. Workstreams not yet landed (see §Remaining work).

### Remaining work — M-series stretch + M6-onwards

Per [lua54-full-conformance-path-2026-06-20.md](../notes/lua54-full-conformance-path-2026-06-20.md):

- **M3-extended** — Table DFS through array + hash parts; per-frame stack walk per ADR 0160; capturing closures as roots.
- **M4-extended** — Builtin migration to `_ENV` entries; free-name fallback to `_ENV[name]`; multi-hop nested closure capture of `_G`.
- **M5-extended** — String → String marshaling (allocator coordination); TaggedValue marshaling; multi-return; error propagation; GC interaction; user-defined bridge crates.
- **M7 String patterns** — ADR 0155 implementation.
- **M8 M1-extended** — Runtime Integer-tagged TaggedValue slot, tostring precision, Phase C i64 ops, integer comparison.
- **M9 `<close>` / `<const>` / `__close` / goto** — Lua 5.4 new syntax.
- **M10 `__gc` / `__mode`** — ADRs 0163 / 0164 implementation; depends on M3-extended.
- **M11 stdlib expansion** — math / table / os / utf8 / debug / basic select.
- **M12 userdata + Bridge extensions** — depends on M5 + M10.
- **M13 Coroutines** — runtime + library.
- **M14 load / require / package** — runtime self-hosting.
- **M15 Phase 4 production hardening + close declaration**.
- **M16 Phase 5 CI / release / security audit**.
- **M17 Lua 5.4 preferred-subset 網羅宣言**.

## Scope (literal)

- ✅ Declare M1-M5 minimum-viable closes recorded in this doc.
- ✅ Pin the test-count progression (1464 → 1519, +55 over 17 sub-ADRs in M1-M5).
- ✅ Defer Phase 4 full close to a future meta-ADR after PRD §7 metrics land.
- ❌ Implement any remaining workstream now. Decision-only ADR.
- ❌ Re-litigate scopes covered by M1-M5 sub-ADRs.
- ❌ Commit to a session schedule for M7-M17.

## Test count delta

```
Step 0:  1519 (after ADR 0226, M5 close)
C1 (this doc): 1519 → 1519 (decision-only, no test)
```

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry meta-ADR.
- [Roadmap revision 2026-06-16](../notes/roadmap-revision-2026-06-16.md) — milestone-based goal framework.
- [Roadmap status 2026-06-20](../notes/roadmap-status-2026-06-20.md) — milestone status table.
- [Lua 5.4 full-conformance path 2026-06-20](../notes/lua54-full-conformance-path-2026-06-20.md) — M-series sequence beyond M6.
- ADRs 0209-0226 — the M1-M5 sub-ADR landed set.
