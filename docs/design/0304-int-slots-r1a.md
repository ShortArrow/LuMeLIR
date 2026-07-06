# 0304. i64 slots — gated first step (F2-R1-a)

- **Status:** Accepted (implements ADR 0303 §R1-a)
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `LocalInfo.int_slot: bool` (default false everywhere; 5 constructor sites).
- ✅ HIR post-pass `mark_integer_slot_eligible` runs after `propagate_number_subtype` on **chunk locals only**. Conservative whole-chunk gate: every top-level stmt must be `LocalInit`/`Assign` with an Integer-literal RHS, or `print(...)` whose args are plain Locals / literals. Any other stmt shape → bail-out, everything stays f64.
- ✅ Codegen: `emit_alloca_int_slot` (single i64); chunk slot map branches on `int_slot`; `LocalInit`/`Assign` store emits the i64 constant directly (gate guarantees literal RHS — `unreachable!` otherwise); Print arm loads i64 + `%lld` ahead of the ADR 0233 fptosi path.
- ✅ Probe fixed: `local big = 9007199254740993; print(big)` now exact (was `...992`); `local m = <maxinteger literal>` exact (was `-9223372036854775808` via fptosi poison).
- ❌ Function-body locals — R1-b/c.
- ❌ Arithmetic on int slots — R1-b (gate bails on any BinOp today).
- ❌ Boundary crossings (call args, TaggedValue, tables) — R1-c.
- ⚠️ The `lumelir run` CLI prepends `arg[]` setup statements (ADR 0115), so CLI-invoked chunks currently bail the gate — direct-compile path (tests, `compile` subcommand output) benefits. Resolved when R1-c drops the gate.

## Tests

5 e2e (`tests/phase4_f2r1a_int_slots.rs`): >2^53 exact; maxinteger literal exact; small ints unchanged; reassignment; gate-bail on arithmetic chunk keeps old behavior. Suite 1824 → 1829.

## References

- ADR 0303 — the design this implements (§R1-a).
- ADR 0233 — prior f64+fptosi print path (still the fallback).
- ADR 0214 — static literal fast path.
