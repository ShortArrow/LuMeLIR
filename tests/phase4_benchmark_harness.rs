//! Phase 4 (ADR 0194) benchmark harness MVP.
//!
//! Compares LuMeLIR-compiled native wall-clock runtime against
//! LuaJIT subprocess runtime for a single micro-benchmark
//! (recursive `fib(N)`). Reports the ratio via `eprintln!` — no
//! perf assertion. Pre-optimisation LuMeLIR is expected to be
//! 10-100x slower; a future ADR adds a regression-detection
//! gate when a baseline exists.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const FIB_N: u32 = 30;
const FIB_EXPECTED: &str = "832040";
const EXPECTED: &str = FIB_EXPECTED;
const ITERATIONS: usize = 5;
const SOURCE: &str = r#"
local function fib(n)
  if n < 2 then return n end
  return fib(n-1) + fib(n-2)
end
print(fib(__N__))
"#;

/// ADR 0206 — string-concat heavy benchmark. Generates a length-N
/// string via repeated `..` concatenation and prints the final
/// length. Exercises a different runtime hot path than `fib`
/// (string-object alloc / memcpy vs recursive-call dispatch).
const CONCAT_N: u32 = 200;
const CONCAT_SOURCE: &str = r#"
local s = ""
local i = 1
while i <= __N__ do
  s = s .. "x"
  i = i + 1
end
print(#s)
"#;

fn run_lumelir(bin: &Path) -> (Duration, String) {
    let t = Instant::now();
    let out = Command::new(bin)
        .output()
        .expect("failed to run compiled binary");
    let elapsed = t.elapsed();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (elapsed, stdout)
}

fn run_luajit(lua_src: &Path) -> (Duration, String) {
    let t = Instant::now();
    let out = Command::new("luajit")
        .arg(lua_src)
        .output()
        .expect("failed to run luajit subprocess");
    let elapsed = t.elapsed();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (elapsed, stdout)
}

fn repeat_and_collect<F: FnMut() -> (Duration, String)>(
    iters: usize,
    mut runner: F,
) -> (Vec<Duration>, String) {
    let mut times = Vec::with_capacity(iters);
    let mut last_stdout = String::new();
    for _ in 0..iters {
        let (d, s) = runner();
        times.push(d);
        last_stdout = s;
    }
    (times, last_stdout)
}

fn median_ms(times: &[Duration]) -> f64 {
    let mut sorted: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

#[test]
fn benchmark_fibonacci_lumelir_vs_luajit() {
    let src = SOURCE.replace("__N__", &FIB_N.to_string());

    // LuMeLIR: compile once.
    let chunk = lumelir::parser::parse(&src).expect("parse fib source");
    let hir = lumelir::hir::lower(&chunk).expect("hir lower fib source");
    let lumelir_bin = std::env::temp_dir().join("lumelir_bench_fib");
    lumelir::codegen::compile(&hir, &lumelir_bin).expect("codegen compile fib");

    let (lumelir_times, lumelir_stdout) =
        repeat_and_collect(ITERATIONS, || run_lumelir(&lumelir_bin));
    let _ = std::fs::remove_file(&lumelir_bin);
    assert_eq!(
        lumelir_stdout, EXPECTED,
        "LuMeLIR fib({FIB_N}) stdout mismatch"
    );

    // LuaJIT availability probe.
    let luajit_avail = Command::new("luajit").arg("-v").output().is_ok();
    if !luajit_avail {
        eprintln!(
            "fib({FIB_N}): LuMeLIR {:.2} ms (median of {ITERATIONS}); luajit not on PATH — comparison skipped",
            median_ms(&lumelir_times)
        );
        return;
    }

    let lua_src_path = std::env::temp_dir().join("lumelir_bench_fib.lua");
    std::fs::write(&lua_src_path, &src).expect("write fib.lua");
    let (luajit_times, luajit_stdout) =
        repeat_and_collect(ITERATIONS, || run_luajit(&lua_src_path));
    let _ = std::fs::remove_file(&lua_src_path);
    assert_eq!(
        luajit_stdout, EXPECTED,
        "LuaJIT fib({FIB_N}) stdout mismatch"
    );

    let lumelir_median = median_ms(&lumelir_times);
    let luajit_median = median_ms(&luajit_times);
    let ratio = lumelir_median / luajit_median;
    let tag = if ratio < 1.0 { "faster" } else { "slower" };
    eprintln!(
        "fib({FIB_N}): LuMeLIR {lumelir_median:.2} ms  LuaJIT {luajit_median:.2} ms  ratio {ratio:.2}x ({tag}) (median of {ITERATIONS})"
    );
}

/// ADR 0206 — second micro-benchmark: string-concat heavy.
/// Same harness structure as the fib test; reports the ratio
/// without asserting on perf.
#[test]
fn benchmark_string_concat_lumelir_vs_luajit() {
    let expected = CONCAT_N.to_string();
    let src = CONCAT_SOURCE.replace("__N__", &CONCAT_N.to_string());

    let chunk = lumelir::parser::parse(&src).expect("parse concat source");
    let hir = lumelir::hir::lower(&chunk).expect("hir lower concat source");
    let lumelir_bin = std::env::temp_dir().join("lumelir_bench_concat");
    lumelir::codegen::compile(&hir, &lumelir_bin).expect("codegen compile concat");

    let (lumelir_times, lumelir_stdout) =
        repeat_and_collect(ITERATIONS, || run_lumelir(&lumelir_bin));
    let _ = std::fs::remove_file(&lumelir_bin);
    assert_eq!(
        lumelir_stdout, expected,
        "LuMeLIR concat({CONCAT_N}) stdout mismatch"
    );

    let luajit_avail = Command::new("luajit").arg("-v").output().is_ok();
    if !luajit_avail {
        eprintln!(
            "concat({CONCAT_N}): LuMeLIR {:.2} ms (median of {ITERATIONS}); luajit not on PATH — comparison skipped",
            median_ms(&lumelir_times)
        );
        return;
    }

    let lua_src_path = std::env::temp_dir().join("lumelir_bench_concat.lua");
    std::fs::write(&lua_src_path, &src).expect("write concat.lua");
    let (luajit_times, luajit_stdout) =
        repeat_and_collect(ITERATIONS, || run_luajit(&lua_src_path));
    let _ = std::fs::remove_file(&lua_src_path);
    assert_eq!(
        luajit_stdout, expected,
        "LuaJIT concat({CONCAT_N}) stdout mismatch"
    );

    let lumelir_median = median_ms(&lumelir_times);
    let luajit_median = median_ms(&luajit_times);
    let ratio = lumelir_median / luajit_median;
    let tag = if ratio < 1.0 { "faster" } else { "slower" };
    eprintln!(
        "concat({CONCAT_N}): LuMeLIR {lumelir_median:.2} ms  LuaJIT {luajit_median:.2} ms  ratio {ratio:.2}x ({tag}) (median of {ITERATIONS})"
    );
}
