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

## Pattern B — Migration carry-over (run 2026-05-07)

The Pattern A1 spike used a trivial `llvm.func` body (declaration
only). Lumelir's real `emit_function` body packs `arith.*` (254
sites), `scf.*` (60), `func.call` (7), and substantial `llvm.*`
ops (already mixed). Pattern B verifies the body content carry-
over by constructing minimal `llvm.func` bodies that exercise
each op family.

| Spike | Body content                                     | `Module::verify()` | `mlir-translate` | clang link + run | Notes                                            |
|-------|--------------------------------------------------|--------------------|------------------|------------------|--------------------------------------------------|
| B1    | `arith.constant`, `arith.addf`, `arith.fptosi`   | PASS               | PASS             | PASS (exit 3)    | `llvm.call @user_fn_add` from `main` also works  |
| B2    | `arith.cmpf`, `scf.if -> f64`, `scf.yield`       | PASS               | PASS             | PASS (exit 7)    | structured control flow lowers to phi cleanly    |
| B3    | `func.call @callee` where `@callee` is `llvm.func` | **FAIL**           | (not reached)    | (not reached)    | verifier: `'func.call' op 'callee' does not reference a valid function` |
| B4    | `llvm.call @callee` (B3 fallback)                | PASS               | PASS             | PASS (exit 5)    | confirms migration target                         |

### Resolved sub-hypotheses

- **Sub-hyp 1 (arith inside `llvm.func`):** PASS. No migration
  needed for the 254 `arith.*` sites.
- **Sub-hyp 2 (scf inside `llvm.func`):** PASS. No migration
  needed for the 60 `scf.*` sites.
- **Sub-hyp 3 (`func.call` to `llvm.func`):** FAIL. `func.call`
  cannot resolve an `llvm.func` symbol — verifier names this
  explicitly. The 7 `func::call @user_fn_NN` sites and 3
  `func::constant @user_fn_NN` sites in `emit.rs` must migrate
  in lockstep with `emit_function`.
- **Sub-hyp 4 (`llvm.call` fallback):** PASS. `llvm.call`
  correctly resolves an `llvm.func` callee; B1 and B4 both
  exercise this path end-to-end.

### B5 — multi-return `llvm.func` (added 2026-05-07)

`__lumelir_next` (ADR 0081, returns `(i64, i64, i64, i64)`) and
every TaggedValue-widened user fn (ADR 0074) currently rely on
the `func` dialect's native multi-result capability. LLVM IR has
no native multi-return; MLIR's `llvm` dialect mirrors that
restriction.

| Probe | Form                                                | Result |
|-------|-----------------------------------------------------|--------|
| B5a   | `llvm.func @pair() -> (i64, i64)` direct multi-result | melior 0.27's `llvm::r#type::function` only accepts a single return type — the multi-result form is not expressible in the API. |
| B5b   | `llvm.func @pair() -> !llvm.struct<(i64, i64)>` with `llvm.return` of a single struct value built via `llvm.mlir.undef` + chained `llvm.insertvalue`; caller extracts via `llvm.extractvalue %s[i]`. | **PASS** — verify, mlir-translate, clang link/run all green; LLVM IR shows `define { i64, i64 } @pair() { ret { i64, i64 } { i64 11, i64 22 } }`. |

### Implication for Commit 2a scope

B5b is the only available form, which expands Commit 2a beyond
a `+20 LOC mechanical replacement` to a structural change:

- Every user fn whose `ret_kinds.len() > 1` (multi-return per
  ADR 0021, TaggedValue-widened per ADR 0074, plus
  `__lumelir_next` per ADR 0081) needs:
  1. function type's return becomes a single
     `!llvm.struct<(t0, t1, ...)>` instead of an N-tuple.
  2. body's terminator builds the struct via `llvm.mlir.undef`
     + N × `llvm.insertvalue`, then `llvm.return %struct`.
  3. every callsite (emit.rs:3233 / 3355 / 3467 / 3967 / 6570
     and the call-multi machinery) extracts each result via
     `llvm.extractvalue %ret[i]` instead of
     `op_ref.result(i)`.
- `flat_result_index` (`callabi.rs`) currently maps each
  TaggedValue position to its 2-result span; after migration
  the mapping shifts to struct-field indices.
- Single-return user fns (the simpler majority) keep the
  trivial `llvm.func ... -> single_ty` shape.

The change is still **behavior-invariant** at the LLVM IR level
(LLVM multi-return is always struct-coded), so 965 tests should
stay green if the mapping stays consistent. The diff size shifts
upward to roughly +150 LOC of careful structural rewrite plus
`extractvalue` at every multi-result callsite, instead of the
~+20 LOC pure-mechanical replacement originally scoped.

Commit 2a is therefore best opened with a small Tidy First helper
(`emit_pack_struct_return` / `emit_extract_struct_result` in
`callabi.rs`), then flipping `emit_function` / `emit_main` /
`emit_lumelir_next_function` / all call+constant sites in
lockstep, finishing with the 4 IR-shape unit-test assertion
updates. Spike branch `spike/closure-mlir` keeps the proof
binaries (commits `976ad8b`, `865f6c6`, `3af1ac7`).

### `func.call` 7-site migration plan

After `emit_function` flips to `LLVMFuncOperationBuilder`, every
`func::call @user_fn_NN` callsite must become `llvm.call`:

| File:line                       | Site                                                |
|---------------------------------|------------------------------------------------------|
| `src/codegen/emit.rs:3233`      | ADR 0082 `emit_dispatch_chain_recursive` direct call |
| `src/codegen/emit.rs:3355`      | direct user fn call                                  |
| `src/codegen/emit.rs:3467`      | direct user fn call                                  |
| `src/codegen/emit.rs:3967`      | `emit_call_user_into_tagged_slot` (ADR 0074)         |
| `src/codegen/emit.rs:6570`      | call site                                            |
| `src/codegen/emit.rs:6631`      | call site                                            |
| `src/codegen/emit.rs:1920`-ish  | `__lumelir_next` call (if reached this site list)    |

The 3 `func::constant @user_fn_NN` sites
(`emit.rs:3181, 6183, 6659`) become `llvm.mlir.addressof` calls,
yielding `!llvm.ptr` directly — the existing
`emit_unrealized_cast` from `func`-typed value to `!llvm.ptr`
disappears on this path.

`func.return` sites (`emit.rs:984, 1033, 1920`) become
`llvm.return`. `__lumelir_next` (ADR 0081) and `emit_main` are
themselves user-fn-shaped, so they migrate alongside.

## Commit 2 go / no-go

**GO criteria** (all met after Pattern A1 + B1 + B2 + B4):
- Pattern A1 verified: `@user_fn_NN_closure` static singleton
  is realisable.
- `arith` and `scf` ops do not need migration.
- `llvm.call` is the proven replacement for `func.call`.

Commit 2 may now land in a single atomic commit. The migration
must touch all of the following in lockstep:

1. `emit_function`: `func::func` → `LLVMFuncOperationBuilder`,
   `func::r#return` → `llvm.return`. ABI-compatible (i64-tag
   wide returns from ADR 0074 and Function-typed returns from
   ADR 0019 stay valid; only the wrapping op kind changes).
2. `__lumelir_next` (ADR 0081) and `emit_main`: same migration.
3. All 7 `func::call @user_fn_NN` callsites → `llvm.call`.
4. All 3 `func::constant @user_fn_NN` callsites →
   `llvm.mlir.addressof`, removing the now-obsolete
   `emit_unrealized_cast` to `!llvm.ptr`.
5. Emit `@user_fn_NN_closure` static globals using Pattern A1.
6. Flip the TAG_FUNCTION producer (closure-creation) to store
   the cell pointer; flip ADR 0082 dispatch chain to load
   `cell.fn_ptr` (offset 0) before the candidate compare.

Steps 1-4 are pure-mechanical and could be split off as a Tidy
First commit if the Commit 2 diff threatens test isolation.
Steps 5-6 are the atomic semantic cutover.

**NO-GO triggers** (none active; recorded for future regression):
- Any future melior/MLIR upgrade that re-fails B1, B2, or B4 → fall
  back to splitcase emission per the "Next experiment" section.

## ADR 0083 Commit 2 entry order

With the static-singleton path open and the migration carry-over
verified, Commit 2 ("TAG_FUNCTION payload semantic cutover") can
land in this order:

1. **Migrate user-fn emission** to `llvm.func`. The body region
   today contains `arith.*` / `scf.*` / `func.call` / `func.return`
   ops. Pattern B confirms `arith` and `scf` stay valid inside
   `llvm.func`; only `func.call`/`func.return` need their LLVM
   counterparts. The terminator changes from `func.return %v`
   to `llvm.return %v`.
2. **Migrate intra-fn callees** from `func::call` to `llvm.call`
   (Pattern B4 verified), and from `func::constant` to
   `llvm.mlir.addressof` where the value is meant as `!llvm.ptr`
   (e.g. ADR 0082's `emit_dispatch_chain_recursive`). The existing
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

### Landed (2026-05-07/08)

- **Commit 2a** (`551d51c`): steps 1 + 2 above — `emit_function`
  / `emit_main` / `emit_lumelir_next_function` flipped to
  `LLVMFuncOperationBuilder`, all `func::call` / `func::constant`
  callees migrated, multi-return wrapped in `!llvm.struct<(...)>`.
  Function-kind value type went from `!func.func<...>` to
  `!llvm.ptr`. Behavior 不変 (965/0).
- **Commit 2a-fix** (`c81f16b`): Codex review found that the
  `!llvm.ptr` Function-value erasure removed an MLIR-verifier
  safety net, exposing silent wrong-code in `Callee::Indirect`'s
  hardcoded `f64` result type. HIR now rejects non-Number
  ret_kinds on Function-kind parameter routes (ADR 0075 amend).
  +5 reject tests (970/0).
- **Commit 2b**: steps 3-5 above. Per-user-fn
  `@user_fn_NN_closure` static globals emitted via
  `closure::emit_user_fn_closure_global` (Pattern A1 +
  `emit_pack_struct`). Producer flip in `HirExprKind::FunctionRef`
  and known-FuncId `Local` paths. Consumer flip in
  `emit_indirect_dispatch_chain_in_block` (payload load → cell
  ptr) and `Callee::Indirect` arm — both insert
  `closure::emit_load_closure_fn_ptr` before the actual call /
  candidate compare. Dispatch-chain candidate side stays at raw
  fn ptr (no double indirection). 3 new IR-shape tests
  (973/0). Singleton property (1 fn = 1 global) preserves Lua
  spec §3.4.4 closure equality without extra work.

## Carry-overs (out of spike scope, but worth flagging)

- **`arith` / `scf` / `func.call` inside `llvm.func` body:**
  RESOLVED 2026-05-07 by the Pattern B sub-spikes above
  (b1/b2/b3/b4 on `spike/closure-mlir`). `arith` and `scf` need
  no migration; `func.call` must flip to `llvm.call` in lockstep
  with `emit_function`'s migration to `LLVMFuncOperationBuilder`.
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
