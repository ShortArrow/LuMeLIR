# 0194. Benchmark Harness MVP — LuMeLIR vs LuaJIT (Recursive Fibonacci)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0193](0193-phase4-entry-criteria.md) §Ordering recommendation §1: "Benchmark harness first — without a measurement floor, performance work is speculative." This ADR is that benchmark harness MVP.

PRD §7 success metric §1 names "単純計算において LuaJIT と同等以上の速度を出す" as the target. Without a reproducible measurement protocol, Phase 4 close criterion §1 ("performance metric satisfied OR explicit-gap rationale") cannot be evaluated. This ADR establishes the protocol with one micro-benchmark (recursive Fibonacci) and one comparison target (LuaJIT 2.1).

Probe confirms baseline feasibility:
- LuMeLIR compiles + runs `fib(20)` → `6765` (recursive call shape works through the codebase's existing call ABI).
- LuaJIT 2.1.1780076327 (already installed at `/usr/sbin/luajit`) runs the same Lua source to the same output.

## Scope (literal)

- ✅ Single micro-benchmark: `fib(N)` recursive Fibonacci. Tunable `N` so the median wall-clock fits in a CI-friendly window.
- ✅ Wall-clock measurement via `std::time::Instant`. Repeat-and-take-median over a small number of iterations (default 5) to dampen outliers.
- ✅ Side-by-side comparison: LuMeLIR-compiled binary vs `luajit` subprocess, same Lua source.
- ✅ Ratio reporting: `lumelir / luajit` as a single floating-point number. No assertion on the value — the test is for **measurement**, not gatekeeping.
- ✅ Correctness assertion: both implementations produce the same `fib(N)` stdout. The harness is meaningless if the values diverge.
- ✅ Skip-when-no-luajit: if `luajit` is not on PATH, the test prints a warning and skips the comparison portion. LuMeLIR-only timing still runs and asserts correctness.
- ❌ Additional benchmarks (prime sieve, string concat, table ops, allocations). Each future micro-benchmark gets its own ADR or extension here when added.
- ❌ Criterion crate / `cargo bench` infrastructure. Standard `#[test]` is sufficient for the MVP; criterion adds dependency + complexity without buying anything the MVP needs.
- ❌ Performance assertions / regression detection. The MVP reports; a future ADR adds a regression-detection gate when historical baselines exist.
- ❌ Statistical significance testing. Median-of-5 is the chosen smoothing; deeper statistics (confidence intervals, t-tests) are out of scope until a real regression-detection gate emerges.
- ❌ Compile-time accounting (how long `lumelir::codegen::compile` itself takes). The MVP measures executed-binary runtime only; compile-time is a separate concern.

## Decision

### Harness shape — `tests/phase4_benchmark_harness.rs`

```rust
//! Phase 4 (ADR 0194) benchmark harness MVP. Compares
//! LuMeLIR-compiled wall-clock runtime against LuaJIT
//! subprocess runtime for a single micro-benchmark.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

const FIB_N: u32 = 30;          // ~10ms on LuaJIT, plenty of signal
const ITERATIONS: usize = 5;
const SOURCE: &str = r#"
local function fib(n)
  if n < 2 then return n end
  return fib(n-1) + fib(n-2)
end
print(fib(__N__))
"#;

#[test]
fn benchmark_fibonacci_lumelir_vs_luajit() {
    let src = SOURCE.replace("__N__", &FIB_N.to_string());

    // Compile LuMeLIR once
    let chunk = lumelir::parser::parse(&src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    let lumelir_bin = std::env::temp_dir().join("lumelir_bench_fib");
    lumelir::codegen::compile(&hir, &lumelir_bin).unwrap();

    let lumelir_times = repeat_and_collect(ITERATIONS, || run_lumelir(&lumelir_bin));
    let _ = std::fs::remove_file(&lumelir_bin);

    let lua_src_path = std::env::temp_dir().join("lumelir_bench_fib.lua");
    std::fs::write(&lua_src_path, &src).unwrap();
    let luajit_avail = Command::new("luajit").arg("-v").output().is_ok();

    if !luajit_avail {
        eprintln!("luajit not on PATH; skipping comparison");
        return;
    }
    let luajit_times = repeat_and_collect(ITERATIONS, || run_luajit(&lua_src_path));
    let _ = std::fs::remove_file(&lua_src_path);

    let lumelir_median = median_ms(&lumelir_times);
    let luajit_median  = median_ms(&luajit_times);
    let ratio          = lumelir_median / luajit_median;

    eprintln!(
        "fib({FIB_N}): LuMeLIR {lumelir_median:.2} ms  LuaJIT {luajit_median:.2} ms  \
         ratio {ratio:.2}x ({})",
        if ratio < 1.0 { "faster" } else { "slower" }
    );
}
```

Helpers (`run_lumelir`, `run_luajit`, `repeat_and_collect`, `median_ms`) are inlined for the MVP.

### `FIB_N = 30` justification

- LuaJIT runtime ≈ 5-15 ms on modern desktop hardware.
- LuMeLIR runtime expected within 1-2 orders of magnitude until optimisation passes land. Comfortable signal window.
- Total test wall-clock (5 iterations × 2 implementations × ~10 ms median): ~100 ms. CI-friendly.

### Output verification

Both implementations must print `fib(30) = 832040` (the actual `print(fib(30))` output is `"832040\n"`). The harness asserts the LuMeLIR output's trimmed stdout matches. The LuaJIT subprocess output is also verified before timing.

### Skip behaviour

`Command::new("luajit").arg("-v").output().is_ok()` detects luajit availability. If absent, the test reports skip via `eprintln!` and returns without `panic!`-ing — same etiquette as `#[ignore]` but lighter touch (no special test-runner argument needed).

### Tests

`tests/phase4_benchmark_harness.rs` (NEW) — one test (the harness itself). This is Red Day 0 vacuously — the test doesn't exist yet; once added, it's a regression net that the harness keeps working as the codebase evolves.

## Alternatives considered

- **Use the `criterion` crate.** Rejected for MVP — adds a dev-dependency, requires `cargo bench` invocation (deviates from the `cargo test` flow the codebase already uses), and produces report formatting heavier than the MVP needs.
- **Skip the harness and write performance ADRs based on intuition.** Rejected — ADR 0193 explicitly named the harness as criterion 1 prereq; intuition-only perf work was rejected in the meta-ADR.
- **Use a richer benchmark suite (Fibonacci + prime sieve + string concat + table ops) for the MVP.** Rejected — overscoped. One benchmark proves the harness; future ADRs extend.
- **Assert a specific ratio threshold** ("LuMeLIR must be ≤ 2x LuaJIT"). Rejected — pre-optimisation LuMeLIR is expected to be 10-100x slower; locking an assertion now would just block CI on a known-failing metric. The PRD §7 target is achieved later via optimisation passes; the harness merely reports until then.

## Consequences

**Positive**
- Phase 4 close criterion §1 has a measurement protocol.
- Future optimisation ADRs have a baseline to compare against.
- LuaJIT availability is detected gracefully; the harness works in luajit-less environments (slower) and luajit-equipped environments (full comparison).
- One file, one test, no new dependencies.

**Negative**
- Single benchmark is not a comprehensive suite. Acceptable — MVP, future ADRs extend.
- Wall-clock measurement is environment-sensitive (other load, frequency scaling). Median-of-5 mitigates but doesn't eliminate.
- The `print` Lua output round-trip dominates a fraction of fib(20)'s runtime; future benchmarks can use longer N to push this fraction down further.

**Locked in until superseded**
- `tests/phase4_benchmark_harness.rs` is the SoT for benchmarking. New benchmarks extend it; new methodology refactors it.
- LuaJIT 2.1 is the comparison target. Future ADR may switch to PUC-Rio Lua 5.4 if the priority shifts.

## Documentation updates

- [x] §8 — adds 0194.
- [x] ADR 0193 §Ordering recommendation §1 — harness done; cross-reference.

## Test count delta

```
Step 0: 1430 (after 64ab25d)
C1 (doc): 1430 → 1430
C2 (skipped — no Red Day 0 separation; harness is itself the test)
C3 (impl): 1430 → 1431
```

## Critical files

- `docs/design/0194-benchmark-harness-mvp.md` (this doc).
- `docs/design/README.md` index entry.
- `tests/phase4_benchmark_harness.rs` (NEW).

## Risks

| Risk | Mitigation |
|---|---|
| `luajit` absent → test panics | Skip-when-absent via `Command::new("luajit").arg("-v")` probe. |
| CI environment runs differently → false ratios | Documented as known limitation; harness reports, doesn't gate. |
| `print(fib(30))` output dominates timing → fib runtime masked | fib(30) ≈ 2.7M recursive calls; print is one syscall. Print fraction is negligible at N=30. |
| Median-of-5 hides multi-modal distributions | Acceptable for MVP; statistical depth is future-ADR work when a regression-gate emerges. |
| LuMeLIR ratio is so much worse than LuaJIT it confuses readers | The eprintln message explicitly tags `faster` vs `slower`; no ambiguity. |

## Future work

- ADR 0195+ — Additional micro-benchmarks (prime sieve, string concat heavy, table ops). Each new benchmark = one extension to `tests/phase4_benchmark_harness.rs`.
- ADR (TBD) — Regression-detection gate (introduces a baseline file + a CI-failing assertion when ratio degrades).
- ADR (TBD) — Optimisation pass ADRs informed by harness data: inlining, escape analysis, constant fold, etc.
- ADR (TBD) — Compile-time accounting (separate harness for `lumelir::codegen::compile` duration).

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry; this ADR implements its §Ordering §1.
- [ADR 0191](0191-rust-lua-bridge-mvp.md) — pattern for `tests/phase3_*` cargo-test integration; same shape used here for `tests/phase4_*`.
- PRD `docs/PRD.jp.md` §7 — perf success metric this harness measures against.
- LuaJIT 2.1 — comparison target (`/usr/sbin/luajit`).
