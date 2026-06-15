# 0206. Benchmark — String-Concat Heavy

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

ADR 0194 §Future work named "Additional micro-benchmarks (prime sieve, string concat heavy, table ops)" as bucket C work. This ADR adds the string-concat benchmark — second test in the existing harness.

## Scope (literal)

- ✅ Add `benchmark_string_concat_lumelir_vs_luajit` to `tests/phase4_benchmark_harness.rs` alongside the existing fib test.
- ✅ Lua source: append `"x"` N times via `..` in a `while` loop; print `#s` (the final length).
- ✅ N = 200 chosen to keep wall-clock in the 0.5-2 ms band (enough signal without bloating CI).
- ✅ Same correctness assertion + LuaJIT skip-when-absent + ratio report shape as the fib test.
- ❌ Prime sieve. Deferred — requires more iteration logic, separate ADR if needed.
- ❌ Table-ops benchmark. Deferred.
- ❌ Perf assertion. The harness reports the ratio; no gate.

## Decision

Add a second `#[test]` function in `tests/phase4_benchmark_harness.rs`. Reuses every primitive (`run_lumelir`, `run_luajit`, `repeat_and_collect`, `median_ms`).

## Observed baseline

On dev machine (WSL2 Arch, debug-built lumelir):

```
concat(200): LuMeLIR 0.76 ms  LuaJIT 1.74 ms  ratio 0.44x (faster) (median of 5)
```

LuMeLIR is again competitive — startup amortization dominates at this scale, same pattern as `fib(30)`.

## Test count delta

```
Step 0: 1452 (after 5aa22b6)
C3 (impl): 1452 → 1453
```

## References

- [ADR 0194](0194-benchmark-harness-mvp.md) — benchmark harness MVP, this ADR's parent.
