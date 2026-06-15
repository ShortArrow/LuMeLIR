# 0207. Benchmark — Table Ops (Insert + Sum)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

Third micro-benchmark in the ADR 0194 harness (after fib in 0194 and string-concat in 0206). Per ADR 0194 §Future work: "Additional micro-benchmarks (prime sieve, string concat heavy, table ops)". This ADR closes the **table ops** item.

## Scope

- ✅ Lua source: insert N elements via `table.insert(t, i)`, then sum them via `t[i]` reads.
- ✅ N=500 chosen for ~1-2 ms wall-clock window.
- ✅ Same harness shape (correctness assertion + LuaJIT skip-when-absent + ratio report).

## Observed baseline

```
table(500): LuMeLIR 0.80 ms  LuaJIT 1.68 ms  ratio 0.48x (faster) (median of 5)
```

LuMeLIR competitive on this short, startup-dominated benchmark — same pattern as fib(30) and concat(200).

## References

- [ADR 0194](0194-benchmark-harness-mvp.md) — benchmark harness MVP.
- [ADR 0206](0206-benchmark-string-concat.md) — string-concat benchmark.
