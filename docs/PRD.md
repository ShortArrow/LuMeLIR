# PRD: LuMeLIR (Lua Multi-Level Intermediate Representation)

> This document is the English translation of [`PRD.jp.md`](./PRD.jp.md), which is the Source of Truth. If the two diverge, the Japanese version wins until re-synced.

## 1. Product Overview

LuMeLIR is a next-generation compiler toolchain that lowers Lua through MLIR (Multi-Level Intermediate Representation) into native executables for heterogeneous targets — CPU, GPU, FPGA, and microcontrollers.

## 2. Target Users

- **Embedded / FPGA engineers** who want to write hardware control logic with Lua's ergonomics.
- **Systems programmers** who want to embed a native-speed scripting environment inside Rust or C++ applications.
- **HPC developers** who want to execute numerical models written in Lua on GPUs or specialized accelerators.

## 3. Problems Addressed

- **LuaJIT's maintainability and extensibility ceiling**: LuaJIT's hand-crafted assembly pipeline is opaque. LuMeLIR leans on MLIR's modularity to make modern optimization passes composable and reusable.
- **Runtime constraints**: Standard Lua is weak on bare-metal embedded and heterogeneous compute (GPU / FPGA) scenarios.
- **Lack of AOT compilation**: There is real demand for fully ahead-of-time compiled Lua binaries that run without a separate runtime.

## 4. Key Features

### 4.1 Defining the LuMeLIR Dialect

Define an MLIR dialect that abstracts Lua's dynamic semantics — tables, metatables, upvalues, coroutines. Operations such as `lumelir.table_set` and `lumelir.call_meta` are implemented at this layer.

### 4.2 Progressive Lowering

- **High-Level**: Optimizations that preserve Lua's dynamic behavior (constant folding, dead-code elimination).
- **Mid-Level**: Loop and array optimizations via the Affine dialect and friends.
- **Low-Level**: Lower to the LLVM dialect and emit the final binary.

### 4.3 Multi-target Backend

- **CPU**: AOT binaries for x86_64, Arm, and RISC-V.
- **MCU**: Bare-metal execution on microcontrollers (e.g. STM32) with a minimized standard library.
- **Acceleration**: Experimental offload paths that route critical compute kernels to GPU (SPIR-V) or FPGA (hardware description).

### 4.4 Rust-based Toolchain

- **Compiler core**: Built in Rust using Melior (MLIR binding).
- **CLI**: Modern interface in the shape of `lumelir compile` and `lumelir run`.

## 5. Technology Stack

| Category | Choice |
| --- | --- |
| Implementation language | Rust |
| IR foundation | MLIR (LLVM project) |
| Parser | Rust-native Lua parser (e.g. nom) |
| Targets | LLVM (CPU), SPIR-V (GPU / Compute) |

## 6. Roadmap (Milestones)

### Phase 1: Proof of Concept (PoC)

- Compile numeric expressions and basic function calls through MLIR into an executable file.
- `print(1 + 2)` runs as a native binary.

### Phase 2: Core Semantics

- Implement the heart of Lua: tables.
- Decide on a GC strategy — static lifetime analysis, or linking a minimal runtime.

### Phase 3: Domain-Specific Features

- **Rust-Lua Bridge**: Call Rust functions inlined at the MLIR level.
- Integrate a dialect for register manipulation targeting embedded use cases.

## 7. Success Metrics

- **Performance**: Match or beat LuaJIT on simple arithmetic.
- **Binary size**: Footprint in the range of a few KB to a few hundred KB — small enough for embedded targets.

---

## Notes

The essence of this PRD is the re-framing: **Lua is no longer merely a scripting language; it is a frontend for MLIR's powerful transformation engine.**

In particular, if Phase 3's inline Rust integration lands, any Rust project can embed Lua-grade flexibility at near-zero overhead.
