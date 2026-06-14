# 0192. Embedded Register-Ops Dialect — Entry Decision

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0189](0189-phase3-entry-criteria.md) §Phase 3 close criteria §2 requires "Embedded register-ops entry ADR exists (implementation optional unless user emerges)". This ADR satisfies that requirement.

PRD §6 (Phase 3) names "組み込み用レジスタ操作方言の統合" as the second Phase 3 theme alongside the Rust-Lua Bridge. The Bridge is the PRD kimo (now satisfied via [ADR 0191](0191-rust-lua-bridge-mvp.md)); the embedded dialect is the secondary theme — same Phase 3 in scope, lower priority by explicit PRD memo. This ADR pins the abstract surface so future implementation work has a clear entry point, without committing to a specific target or producing code now.

## Decision

### Surface — what an embedded register-ops dialect is

A **register-ops dialect** for LuMeLIR is an MLIR dialect that exposes target-specific memory-mapped register operations as language-level constructs. The minimum-viable surface consists of:

- **Volatile load / store** to memory-mapped addresses, distinct from ordinary Lua Table/array reads/writes so the optimiser cannot reorder or fuse them.
- **Bit-field read / modify / write** for register sub-ranges (common pattern: 8-bit fields inside a 32-bit register).
- **Per-target lowering** to either inline assembly or LLVM intrinsics that the target backend recognises (e.g. `llvm.read_register` / `llvm.write_register`, MMIO load/store with `volatile` attribute).

User-facing Lua API (illustrative, not yet locked):

```lua
local r = mmio.read(0x4002_1000)    -- volatile read
mmio.write(0x4002_1000, r | 0x01)   -- volatile write
mmio.bits.write(0x4002_1004, 4, 3, 0b101)  -- write bits [4..7) = 5
```

### Target

**Deferred — no target picked yet.** PRD did not name a specific MCU family. Candidates when an embedded user emerges:

| Candidate | Why |
|---|---|
| ARM Cortex-M (Thumbv7-M / Thumbv6-M) | Dominant in commercial embedded; rich LLVM backend; many open-source HALs to bridge against |
| RISC-V (rv32imac) | Growing share; permissively licensed; LLVM backend mature for `rv32` |
| Embedded Linux ARMv7/ARMv8 | Closer to current dev environment (WSL2/Linux) but not the kimo "組み込み" claim |

The MVP target will be whichever an actual user requests first. Until then, the dialect design lives at the abstract level (volatile semantics + bit-field operations) without an LLVM lowering.

### Why decision-only — implementation deferred

Per ADR 0189 close criterion §2: "implementation optional unless user emerges". No embedded user has surfaced; ADR 0191 already proves the kimo (Rust-Lua Bridge). Pursuing speculative MCU codegen ahead of demand would:

- Burn ADR sequencing budget on a target that may never be needed.
- Conflict with the Codex 第3原則 (anti-ad-hoc): designing for "any embedded target" without a concrete consumer is the textbook over-engineering shape.

The right shape for now is **abstract surface + deferred lowering**. When a user emerges, a follow-up ADR picks the target, defines the LLVM intrinsic mapping, and adds e2e tests.

### Phase 3 close criterion §2 — satisfied

With this ADR recorded, ADR 0189 §2 is satisfied. Implementation is explicitly deferred to a future ADR triggered by user demand.

## Scope (literal)

- ✅ Pin the abstract MVP surface (volatile load/store, bit-field rmw) so future work has a starting point.
- ✅ Document candidate targets (Cortex-M, RISC-V, embedded Linux) with criteria for selection.
- ✅ Defer LLVM lowering, HIR builtins, codegen emit arms, and e2e tests to a future user-triggered ADR.
- ❌ Implement any of the above now. No code lands in this ADR.
- ❌ Pick a target. The pick happens when a user emerges, not before.
- ❌ Define the exact Lua surface — the `mmio.<op>` shape above is illustrative only; the user's actual API constraints (`bare`-metal vs HAL-like) drive the final surface.

## Alternatives considered

- **Skip the entry ADR; proceed to Phase 3 close without it.** Rejected — ADR 0189 §2 explicitly requires an entry ADR. Skipping muddies close criteria.
- **Pick Cortex-M now and start implementation.** Rejected — speculative. No user demand, no test path, no compelling reason to consume ADR sequencing budget on this rather than other Phase 4 candidates.
- **Bundle Embedded with Bridge as one Phase 3 deliverable.** Rejected — the two are independent themes with different consumer profiles. PRD memo names them separately; ADR 0189 sequences them separately.
- **Promote `mmio.*` to a `Builtin::*` family analogous to `rust.*` (ADR 0191).** Rejected — the `Builtin::Rust*` shape works for `extern "C"` Rust functions because the ABI is well-defined; embedded register-ops need per-target lowering decisions that the Builtin path doesn't accommodate. Future ADR introduces the right dispatch mechanism.

## Consequences

**Positive**
- ADR 0189 close criterion §2 satisfied with one ADR + one commit.
- Phase 3 close path now needs zero additional ADRs beyond this; close declaration ADR follows.
- Future embedded work has a documented starting point and target-selection rubric.

**Negative**
- The Embedded theme of PRD Phase 3 ships as documentation only. Acceptable — explicit PRD priority allows this.
- A specific embedded user might find the deferral frustrating. Mitigation: clear path from "user appears" → "follow-up ADR".

**Locked in until superseded**
- "Embedded register-ops dialect implementation is user-triggered" is the contract. A future ADR replaces this when triggered.
- The abstract MVP surface (volatile + bit-field) is the minimum to satisfy; expansion (DMA control, interrupt vectors, etc.) is each its own ADR.

## Documentation updates

- [x] §8 — adds 0192.
- [x] ADR 0189 §2 — now satisfied; cross-reference here.

## Test count delta

```
Step 0: 1430 (after 24c4b10)
C1 (this doc): 1430 → 1430 (decision-only, no test)
```

## Critical files

- `docs/design/0192-embedded-register-ops-entry.md` (this doc).
- `docs/design/README.md` index entry.

## Risks

| Risk | Mitigation |
|---|---|
| User demand never materialises and the entry ADR remains unconsumed | Acceptable — ADR 0189 §2 explicitly allows this. The decision-only ADR is the contractually correct shape. |
| Future ADR picks a target that the abstract surface here did not anticipate | The surface is intentionally minimal (volatile + bit-field). Target-specific extension is the future ADR's scope. |
| Embedded user demands hard-real-time guarantees (interrupt latency, GC-free code paths) | Documented constraints; the future implementation ADR addresses them at design time. v1 surface defers everything beyond MMIO read/write. |
| Project takes Phase 4 turn (perf / binary size) before embedded user emerges | Acceptable — Phase 4 can proceed in parallel; embedded work remains as an open future-work bullet. |

## Future work

- ADR (TBD) — Phase 3 close declaration (final meta-ADR; mirror of ADR 0133's Phase 2 close style).
- ADR (TBD, user-triggered) — Target-specific embedded register-ops implementation. Picks Cortex-M or RISC-V or Linux embedded based on which user request lands first. Defines: LLVM intrinsic mapping; HIR `Builtin::Mmio*` variant family (or alternative dispatch); codegen emit arms; e2e tests on a target emulator (QEMU for ARM/RISC-V).
- ADR (TBD) — Bridge sub-pieces (type marshaling, error propagation, GC interaction) per ADR 0191 §Future work. Sequenced when a Bridge consumer needs them, not pre-empted by this Embedded ADR.

## References

- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 entry; this ADR satisfies §2.
- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Rust-Lua Bridge MVP (Phase 3 theme #1, PRD kimo).
- PRD `docs/PRD.jp.md` §6 — names embedded register-ops dialect as Phase 3 theme #2.
- PRD `docs/PRD.jp.md` § "メモ" — explicit kimo priority on the Bridge over Embedded.
- LLVM `llvm.read_register` / `llvm.write_register` intrinsics — likely lowering target for future implementation.
