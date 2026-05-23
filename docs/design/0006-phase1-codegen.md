# 0006. Phase 1 Codegen: Standard MLIR Dialects, No HIR, melior 0.27

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

## Context

Phase 1 targets a single expression: `print(1 + 2)` → native binary.
The lexer (8 tests) and parser (11 tests) are complete. This ADR records
the codegen design decisions validated by a melior spike (`~/melior-hello/`).

## Decision

### 1. Standard dialects only (no custom LuMeLIR dialect)

Phase 1 uses `arith`, `func`, and `llvm` dialects from MLIR's standard
library. A custom `lumelir` dialect is deferred to Phase 2+ when
table/scope/name-resolution semantics demand custom ops.

**Rationale:** The AST is flat (Number, BinOp, Call) and maps directly
to `arith.constant`, `arith.addf`, and `llvm.call @printf`. Introducing
a custom dialect now would add complexity without benefit.

### 2. HIR/MIR skip

Codegen consumes `parser::Expr` directly. HIR (high-level IR with
resolved names, scopes, types) is unnecessary while the AST has no
scoping, no variables, and no type system.

**When to introduce HIR:** Phase 2, when variables, tables, or multi-
statement programs arrive.

### 3. print via C printf

`print(x)` emits `llvm.call @printf` with `"%g\n"` format string.
`%g` suppresses trailing zeros (`3.0` → `"3"`), matching Lua semantics.

The format string is an `llvm.mlir.global internal constant`. The binary
links against libc (`-lc`). Freestanding output is Phase 2+.

### 4. melior 0.27 + MLIR 22

- **Crate:** `melior 0.27` with `ods-dialects` feature.
- **MLIR/LLVM:** System packages (Arch `llvm 22.1.3` + AUR `mlir 22.1.3`).
- **Env vars:** `MLIR_SYS_220_PREFIX`, `LLVM_SYS_220_PREFIX`,
  `TABLEGEN_220_PREFIX` — all set to `/usr`.
- **Linking:** Shared mode (`libMLIR.so`, `libMLIR-C.so`, `libLLVM-22.so`).
  `libMLIR-C.so` was manually assembled from static CAPI archives since
  the Arch `mlir` package only ships `.a` files.

### 5. Lowering pipeline (external tools)

```
emit_module() → MLIR text
  → mlir-opt --convert-arith-to-llvm --convert-func-to-llvm --reconcile-unrealized-casts
  → mlir-translate --mlir-to-llvmir
  → llc -filetype=obj -relocation-model=pic
  → cc -o <output> <obj> -lc
```

Phase 2+ may replace external tool invocations with in-process pass
manager and LLVM API calls.

### 6. unsafe usage

No `unsafe` in the codegen module. All unsafe is confined to the
`melior` and `mlir-sys` crates.

## Consequences

- Phase 1 codegen is ~200 lines of Rust (emit + lower + link).
- 8 new unit tests + 2 integration tests validate the pipeline.
- Adding new binary operators (Sub, Mul, Div) requires only adding
  a match arm in `emit_binop`.
- The external-tool lowering pipeline adds ~300ms latency but avoids
  linking complexity; acceptable for Phase 1.
