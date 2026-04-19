# LuMeLIR

> Lua → MLIR → CPU / GPU / FPGA / MCU — a Rust-based AOT compiler toolchain.

**Status:** Early development (Phase 0: scaffolding).

## What is LuMeLIR?

LuMeLIR (Lua Multi-Level Intermediate Representation) lowers Lua through MLIR into native binaries for heterogeneous targets. The design goal is to re-frame Lua as a frontend for MLIR's transformation engine so that modern optimization and codegen work (LLVM, SPIR-V, bare-metal MCU) becomes reusable instead of bespoke.

## Quick Start

```bash
cargo build --release
./target/release/lumelir --help
./target/release/lumelir compile examples/hello.lua -o hello     # (Phase 1+)
./target/release/lumelir run examples/hello.lua                   # (Phase 1+)
```

Today the CLI exists but compilation is stubbed until Phase 1 lands — `compile` / `run` print "not yet implemented: Phase 1 PoC".

## Roadmap

- **Phase 1 — PoC**: AOT-compile `print(1 + 2)` through MLIR into a native binary.
- **Phase 2 — Core Semantics**: Tables, metatables, and a GC strategy.
- **Phase 3 — Domain-Specific Features**: Rust-Lua bridge (MLIR-level inlining), register-manipulation dialect for embedded targets.

See [`docs/PRD.md`](docs/PRD.md) for the full product requirements.

## Documentation

- [`docs/PRD.md`](docs/PRD.md) — Product Requirements (English)
- [`docs/PRD.jp.md`](docs/PRD.jp.md) — Product Requirements (Japanese, Source of Truth)
- [`docs/README.jp.md`](docs/README.jp.md) — Japanese README
- [`docs/design/`](docs/design/) — Architecture Decision Records (ADR)
- [`docs/handover/`](docs/handover/) — Session handover notes (e.g. environment migrations)
- [`AGENTS.md`](AGENTS.md) — Working conventions for LLM coding agents (also the detailed reference for human contributors)
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Human contributor quick start

## Requirements

- Rust **≥ 1.85** (this crate uses the 2024 edition)
- Additional dependencies (Melior / MLIR / LLVM) will be introduced alongside Phase 1

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
