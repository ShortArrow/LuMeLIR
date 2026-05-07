# Closure MLIR Feasibility Spike (ADR 0083 unblock)

- **Date:** 2026-05-07
- **melior version:** 0.27 (`Cargo.toml`)
- **MLIR / LLVM version:** v22 (TBL_GEN_220_PREFIX, MLIR_SYS_220_PREFIX,
  LLVM_SYS_220_PREFIX)
- **Branch:** `spike/closure-mlir` (not merged to `main`)

## Context

ADR 0083 (Phase 2.5c-full closures) shipped only Commit 1 — the
`src/codegen/closure.rs` skeleton — and was tagged "accepted,
deferred" in `AGENTS.md`. The remaining commits (TAG_FUNCTION
payload semantic cutover; captured-local upvalue boxes) need a
**static singleton closure cell per non-capturing user function**:

```text
@user_fn_NN_closure : !llvm.struct<(ptr, i64)> = {
    fn_ptr   = &user_fn_NN,
    upvalue_count = 0,
}
```

The technical risk Codex pre-ADR-0083 review flagged was whether
melior 0.27 actually accepts a non-empty `initializer(Region)` body
on `llvm.mlir.global` referring to a function symbol via
`llvm.mlir.addressof`. That question blocked any production
attempt — a wrong cutover would break the existing 944 → 965
test suite atomically.

This note records the spike result that closes that question.

## Spike scope

Two minimal melior 0.27 examples, both running through the
existing `lumelir::codegen::lower::to_llvm_ir` pipeline
(`mlir-opt --convert-* --reconcile-unrealized-casts` →
`mlir-translate --mlir-to-llvmir`):

- **Pattern A1** — user function emitted as `llvm.func`, closure
  cell built with `llvm.mlir.addressof @user_fn_42` inside an
  `llvm.mlir.global` initializer region.
- **Pattern A2** — identical to A1 except the user function is
  emitted as `func.func` (lumelir's existing style for user
  functions; see `emit_function`).

Each example verifies in three stages:

1. `module.as_operation().verify()` (recursive MLIR verify).
2. `mlir-opt + mlir-translate --mlir-to-llvmir` (the existing
   lumelir lowering pipeline).
3. `clang` link of the LLVM IR + a tiny C stub providing
   `print_addr` / `user_fn_42`; binary expected to exit 0 and
   print a non-zero `@closure_singleton` address.

Spike code: `spike/closure-mlir` branch — `examples/spike_pattern_a.rs`,
`examples/spike_pattern_a2.rs`. Run with
`cargo run --example spike_pattern_a` / `_a2`.

## Pattern A1 — `llvm.func` + `llvm.mlir.addressof` (PASSES)

### Generated MLIR

```mlir
module {
  llvm.func @user_fn_42(f64) -> f64
  llvm.mlir.global internal @closure_singleton() {addr_space = 0 : i32}
      : !llvm.struct<(ptr, i64)> {
    %0 = llvm.mlir.addressof @user_fn_42 : !llvm.ptr
    %1 = llvm.mlir.constant(0 : i64) : i64
    %2 = llvm.mlir.undef : !llvm.struct<(ptr, i64)>
    %3 = llvm.insertvalue %0, %2[0] : !llvm.struct<(ptr, i64)>
    %4 = llvm.insertvalue %1, %3[1] : !llvm.struct<(ptr, i64)>
    llvm.return %4 : !llvm.struct<(ptr, i64)>
  }
  llvm.func @print_addr(!llvm.ptr)
  llvm.func @main() -> i32 {
    %0 = llvm.mlir.addressof @closure_singleton : !llvm.ptr
    llvm.call @print_addr(%0) : (!llvm.ptr) -> ()
    %1 = llvm.mlir.constant(0 : i32) : i32
    llvm.return %1 : i32
  }
}
```

### Lowered LLVM IR (after `mlir-translate`)

```llvm
@closure_singleton = internal global { ptr, i64 } { ptr @user_fn_42, i64 0 }
declare double @user_fn_42(double)
declare void @print_addr(ptr)
define i32 @main() {
  call void @print_addr(ptr @closure_singleton)
  ret i32 0
}
```

The `@closure_singleton` line is the gold result for ADR 0083:
the function pointer is baked in at compile time — no runtime
populate is needed for non-capturing closures.

### Verify result

| Stage                        | Result |
|------------------------------|--------|
| `Module::verify()`           | PASS   |
| `mlir-opt + mlir-translate`  | PASS   |
| `clang` link + run, exit 0   | PASS   |

The runtime stdout was `addr=0x654ab02a5018` — a real, non-zero
pointer to the static cell, confirming end-to-end reachability.

## Pattern A2 — `func.func` + `llvm.mlir.addressof` (FAILS)

### Verify result

| Stage                        | Result |
|------------------------------|--------|
| `Module::verify()`           | **FAIL** |
| `mlir-opt + mlir-translate`  | (not reached) |
| `clang` link + run           | (not reached) |

### Error message (verbatim)

```
error: 'llvm.mlir.addressof' op must reference a global defined by
'llvm.mlir.global', 'llvm.mlir.alias' or 'llvm.func' or
'llvm.mlir.ifunc'
```

The verifier's allow-list is explicit: `func.func` symbols are not
addressable from `llvm.mlir.addressof`. ADR 0083's static-singleton
plan therefore requires user functions to be emitted as `llvm.func`.

## Conclusion

- **Keep:** Pattern A1 — emit non-capturing closures as
  `llvm.mlir.global @user_fn_NN_closure` with a computed
  initializer that calls `llvm.mlir.addressof @user_fn_NN`. The
  pattern verifies end-to-end and produces the canonical static
  singleton in LLVM IR.
- **Reject:** Pattern A2. `llvm.mlir.addressof` cannot resolve
  `func.func` symbols; the verifier message names the exact
  allowed kinds.
- **Implementation directive for ADR 0083 Commit 2:**
  migrate user function emission in `src/codegen/emit.rs::emit_function`
  from `func::func` (producing `func.func`) to
  `LLVMFuncOperationBuilder` (producing `llvm.func`). This
  parallels the existing emission of libc decls
  (`emit_strlen_decl`, `emit_string_runtime_decls`, etc.) which
  already use `LLVMFuncOperationBuilder` for `printf` / `malloc`
  / `free`.
- **Patterns B (zero-init + main populate) and C (heap-only) were
  not exercised** — Pattern A1 succeeded on the first try. Both
  remain documented fallbacks if a future melior or MLIR version
  regresses this path.

## ADR 0083 Commit 2 entry order

With the static-singleton path open, Commit 2 ("TAG_FUNCTION
payload semantic cutover") can land in this order:

1. **Migrate user-fn emission** to `llvm.func`. The body region
   today contains `arith.*` / `scf.*` / `func.call` / `func.return`
   ops; these all remain valid inside an `llvm.func` body
   (`llvm.func` permits any dialect inside its region — only the
   *symbol kind* is restricted). The terminator changes from
   `func.return %v` to `llvm.return %v`.
2. **Migrate intra-fn callees** from `func::call` to `llvm.call`,
   and from `func::constant` to `llvm.mlir.addressof` where the
   value is meant as `!llvm.ptr` (e.g. ADR 0082's
   `emit_dispatch_chain_recursive`). The existing
   `unrealized_conversion_cast` from function-typed value to
   `!llvm.ptr` becomes unnecessary on this path.
3. **Emit `@user_fn_NN_closure` static globals** for every
   non-capturing user function, using Pattern A1's initializer
   region pattern.
4. **Switch the TAG_FUNCTION producer** (closure-creation site;
   ADR 0071 / 0082 currently store raw fn ptr) to store the
   closure cell pointer instead. This is the atomic moment —
   producer and consumer (ADR 0082's dispatch chain that loads
   `cell.fn_ptr`) must flip in the same commit.
5. **Update ADR 0082's dispatch chain** to load `cell.fn_ptr`
   (offset 0) before comparing against candidate function
   addresses. The candidate address comes from
   `llvm.mlir.addressof @user_fn_NN`, identical to what the
   `@user_fn_NN_closure` initializer uses.

If steps 1 + 2 turn out to break tests when applied alone (e.g. an
`arith` op rejected inside `llvm.func` body — currently believed to
be allowed but unverified), they can land as a Tidy First commit
ahead of step 3, splitting the migration from the semantic
cutover.

## Carry-overs (out of spike scope, but worth flagging)

- **`arith` / `scf` / `func.call` inside `llvm.func` body:** the
  spike used a trivial body. Whether the full lumelir codegen
  output (with `arith.cmpi` / `scf.if` / `func.call` to other
  user fns) verifies inside an `llvm.func` body is unverified.
  Quickest follow-on spike: take a passing existing test, replace
  one `func::func` emission with `LLVMFuncOperationBuilder` and a
  `llvm.return` terminator, run the full pipeline. Likely a
  one-line + one-terminator change.
- **TaggedValue upvalues:** ADR 0083 defers these (closure.rs
  line 27-28). When a captured local has `ValueKind::TaggedValue`,
  the 8-byte upvalue box widens to 16 bytes (tag + payload) and
  `UPVALUE_BOX_SIZE` recomputes. This is orthogonal to the
  static-singleton question.
- **Closure equality semantics:** Lua spec says `f == g` compares
  closure identity — Pattern A1 makes every non-capturing
  function alias the same static cell automatically (single
  instance per `@user_fn_NN`), so identity is `addr_eq` of the
  cell pointer. Capturing closures get a fresh cell per
  allocation, also `addr_eq`. Both compose with ADR 0082's
  dispatch chain and ADR 0066's tagged-eq path without further
  work.

## Next experiment (if the carry-over above blocks)

If the `llvm.func`-bodied user functions don't accept
`arith` / `scf` / `func.call` ops, the cleanest fallback is a
*splitcase* emission: keep `func.func @user_fn_NN` for the body
(unchanged), and add a parallel `llvm.func @user_fn_NN_lift`
that wraps it via `func.call` from inside an `llvm.func` body.
The static cell then references `@user_fn_NN_lift`. Cost: one
extra wrapper per function and a single `func.call` indirection.
Acceptable for ADR 0083; revisit when the migration carry-over
spike is run.
