# 0225. Bridge Bool ↔ Bool Marshaling — `rust.invert`

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M5 sub-ADR. [ADR 0224](0224-bridge-string-marshaling.md) widened the bridge to String args; this ADR widens to Bool — args and returns. Together they validate the per-kind marshaling pattern: each new Lua value kind needs an ABI shape decision and a codegen emit arm that matches it.

The Lua name `rust.not` would be a Lua-spec violation — `not` is a reserved word (unary operator) and can't appear as an identifier in any position. The bridge method name is `invert`; the Rust extern remains `rust_not`.

## Scope (literal)

- ✅ New `Builtin::RustNot` HIR variant. `rust_from_method("invert") → Some(RustNot)`. Name `"rust.invert"`, arity `(1, 1)`, `ret_kinds = [Bool]`, `param_kinds_for_arity = [Bool]`. `infer_kind → Bool`.
- ✅ Bridge runtime: `extern "C" fn rust_not(b: bool) -> bool { !b }`.
- ✅ Codegen extern declaration: `llvm.func @rust_not(i8) -> i8`. C ABI for `bool` on x86_64-linux is i8.
- ✅ Emit arm: `llvm.zext` from i1 (Lua Bool storage) → i8 at the call site; `llvm.call @rust_not(i8) -> i8`; `llvm.trunc` from i8 back to i1 on the return.
- ✅ Result composes into Lua control flow: `if rust.invert(x) then ... end` works because the truncated i1 is the standard Bool ABI for predicates.
- ❌ Multi-arg Bool functions (`rust.xor(a, b) -> Bool`). Mechanical extension; future.
- ❌ Mixed-kind args / returns within the same Rust function (e.g. Bool + String args). Future ADR per kind-pair as needed.
- ❌ Nil marshaling. Lua Nil is a distinct kind from Bool in this implementation; no current Rust use case calls for it.
- ❌ Avoiding the i1↔i8 ABI hop (LLVM `i1` zeroext attribute would suppress the explicit zext/trunc but is target-specific). Current shape favors correctness + portability over a single-instruction optimisation.

## Decision

### ABI shape

| Direction | Lua MLIR type | C ABI | Bridge step |
|---|---|---|---|
| Lua → Rust arg | i1 | i8 (`bool`) | `llvm.zext i1 → i8` at call site |
| Rust → Lua ret | i8 (`bool`) | i1 | `llvm.trunc i8 → i1` after call |

The explicit zext/trunc lives in the `Callee::Builtin(Builtin::RustNot)` emit arm. Future Bool args / returns from other bridge functions repeat this two-instruction pattern, or — when N bridge fns ship — a small helper extracts it.

### Why `invert`, not `not`

Lua 5.4 §3.1 lists `not` as a reserved word. The parser rejects it as an identifier in any position, including a `name = exp` table-field shape or `recv.name` index key. The user-facing Lua name picks `invert` for clarity; the Rust symbol stays `rust_not` because no Lua-side syntax sees that name.

## Tests

`tests/phase4_bridge_not.rs` (NEW, 4 e2e):

1. `print(rust.invert(true))` → `"false"`.
2. `print(rust.invert(false))` → `"true"`.
3. Double-NOT round-trip: `not(not(x)) == x` for both `true` and `false`.
4. `if rust.invert(false) then print("taken") else print("missed") end` → `"taken"` — proves the Bool flows into Lua control flow.

## Test count delta

```
Step 0:  1510 (after ADR 0224)
C3 (impl + 4 e2e): 1510 → 1514
```

## References

- [ADR 0191](0191-rust-lua-bridge-mvp.md) — Bridge MVP.
- [ADR 0224](0224-bridge-string-marshaling.md) — String → Number marshaling sibling.
- [Lua 5.4 §3.1](https://www.lua.org/manual/5.4/manual.html#3.1) — reserved words (motivates `invert` vs `not`).
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M5 milestone.
