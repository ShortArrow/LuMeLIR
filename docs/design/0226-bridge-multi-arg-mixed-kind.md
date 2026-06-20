# 0226. Bridge Multi-Arg Mixed-Kind Marshaling — `rust.starts_with`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third (closing) M5 sub-ADR. [ADR 0224](0224-bridge-string-marshaling.md) widened to String args; [ADR 0225](0225-bridge-bool-marshaling.md) widened to Bool. Real-world bridge use combines them: a Rust function accepts multiple Lua values of different kinds and returns a third kind. This ADR proves the composition with `rust.starts_with(s: String, prefix: String) -> Bool`.

The combination matters because each new bridge signature is mostly mechanical once the per-kind ABI shapes are nailed down. M5 closes by demonstrating the composition pattern works without ad-hoc per-signature plumbing.

## Scope (literal)

- ✅ New `Builtin::RustStartsWith` HIR variant. `rust_from_method("starts_with") → Some(RustStartsWith)`. Name `"rust.starts_with"`, arity `(2, 2)`, `ret_kinds = [Bool]`, `param_kinds_for_arity = [String, String]`. `infer_kind → Bool`.
- ✅ Bridge runtime: `extern "C" fn rust_starts_with(s_ptr: *const u8, prefix_ptr: *const u8) -> bool`. Reads both length headers (ADR 0112 layout), short-circuits when `prefix_len > s_len`, byte-compares the first `prefix_len` data bytes.
- ✅ Codegen extern: `llvm.func @rust_starts_with(!llvm.ptr, !llvm.ptr) -> i8`.
- ✅ Emit arm: pass both String ptrs through unchanged (they're already the boxed-object pointers); `llvm.call`; `llvm.trunc` i8 → i1.
- ✅ Composes into `if rust.starts_with(...) then ... end` — the i1 return flows into Lua control flow.
- ❌ String → String marshaling. Returning a Lua string still requires allocator coordination; deferred.
- ❌ Number + Bool / Number + String mixed args. Different combinations; each future bridge fn picks its kind tuple and reuses the existing per-kind ABI shape.
- ❌ Multi-return bridge functions. The C ABI for multi-return is platform-specific; deferred.
- ❌ Error propagation across the boundary (Rust panic → Lua error). ADR 0191 §"Error propagation" deferred contract still active.

## Decision

### ABI composition

Each kind contributes its ABI shape:

| Kind | Args in | Returns | Bridge step |
|---|---|---|---|
| String (ADR 0224) | ptr | — | direct ptr passthrough |
| Bool (ADR 0225) | i8 | i8 | `zext i1 → i8` in / `trunc i8 → i1` out |

`rust.starts_with` instantiates [String, String] → Bool:
- Both args: direct ptr passthrough (no zext/trunc needed for the String side).
- Return: `trunc i8 → i1` (only step at the boundary).

The emit arm is the linear composition. Future bridge fns of the form [K0, K1, ...] → R follow the same pattern — emit each arg's per-kind step, call, emit the return's per-kind step.

### Rust-side comparison

`#![no_std]` precludes `memcmp` import; the Rust function does a small byte-loop. For typical short prefixes (file extensions, scheme prefixes, etc.) this is faster than a libc call due to inline-ability. Long prefixes would benefit from `memcmp` but are out of scope for the demo.

## Tests

`tests/phase4_bridge_starts_with.rs` (NEW, 5 e2e):

1. Matching prefix → `true`.
2. Non-matching prefix → `false`.
3. Empty prefix → `true` (vacuous case).
4. Prefix longer than s → `false` (short-circuit).
5. `if rust.starts_with(...) then ... end` flows correctly.

## Test count delta

```
Step 0:  1514 (after ADR 0225)
C3 (impl + 5 e2e): 1514 → 1519
```

## M5 milestone close

ADRs 0224 + 0225 + 0226 land the M5 minimum-viable close at 3/2-3 sub-ADRs. Per-kind ABI shapes for String (ptr-passthrough), Bool (i1↔i8 hop), and Number (ADR 0191 baseline) are in place; multi-arg mixed-kind composes them. Remaining M5-stretch work:

- String → String marshaling (allocator coordination).
- TaggedValue marshaling (runtime tag dispatch at the Rust side).
- Multiple Number args with Number return — mechanical, deferred until first user demand.
- Error propagation (Rust panic → Lua error, pairs with pcall).
- GC-managed Lua values held by Rust (pairs with ADR 0160 stack walk).
- User-defined bridge crates (drop-in extensibility — ADR 0191 §Future).

## References

- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP.
- [ADR 0224](0224-bridge-string-marshaling.md) — String ABI shape.
- [ADR 0225](0225-bridge-bool-marshaling.md) — Bool ABI shape.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string-object layout.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M5 milestone.
