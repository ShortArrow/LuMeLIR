# LuMeLIR

> Lua → MLIR → CPU / GPU / FPGA / MCU。Rust 製の AOT コンパイラ・ツールチェーン。

**ステータス:** 初期開発（Phase 0: スキャフォールディング）。

## LuMeLIR とは

LuMeLIR (Lua Multi-Level Intermediate Representation) は、Lua を MLIR 経由で多様なターゲット（CPU / GPU / FPGA / マイコン）向けのネイティブバイナリへ落とし込むコンパイラ・ツールチェーンである。Lua を「スクリプト言語」としてではなく「MLIR 変換エンジンのフロントエンド」として再定義し、LLVM / SPIR-V / ベアメタル向けの現代的な最適化とコード生成を再利用可能にすることを狙いとする。

## Quick Start

```bash
cargo build --release
./target/release/lumelir --help
./target/release/lumelir compile examples/hello.lua -o hello     # (Phase 1 以降)
./target/release/lumelir run examples/hello.lua                   # (Phase 1 以降)
```

現時点では CLI の骨格のみ実装されており、`compile` / `run` は Phase 1 着手までスタブである（`not yet implemented: Phase 1 PoC` を出力する）。

## ロードマップ

- **Phase 1 — PoC**: `print(1 + 2)` を MLIR 経由でネイティブバイナリへ AOT コンパイル。
- **Phase 2 — Core Semantics**: Table / metatable の実装と GC 戦略の確定。
- **Phase 3 — Domain Specific Features**: Rust-Lua ブリッジ（MLIR レベルでのインライン呼び出し）、組み込み向けレジスタ操作方言の統合。

詳細は [`docs/PRD.jp.md`](PRD.jp.md) を参照。

## ドキュメント

- [`docs/PRD.jp.md`](PRD.jp.md) — 製品要求仕様（日本語・Source of Truth）
- [`docs/PRD.md`](PRD.md) — 製品要求仕様（英語訳）
- [`../README.md`](../README.md) — 英語版 README
- [`docs/design/`](design/) — アーキテクチャ決定記録 (ADR)
- [`docs/handover/`](handover/) — セッション引き継ぎメモ（環境移行等）
- [`../AGENTS.md`](../AGENTS.md) — LLM エージェント向け作業規範（人間向け詳細もここ）
- [`../CONTRIBUTING.md`](../CONTRIBUTING.md) — 人間 contributor クイックスタート

## 必要環境

- Rust **1.85 以降**（本クレートは 2024 edition を使用）
- Phase 1 着手時に Melior / MLIR / LLVM 等の追加依存を導入予定

## ライセンス

以下のいずれかを選択して利用可能:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) または <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) または <http://opensource.org/licenses/MIT>)

### コントリビューション

あなたが明示的に別段の定めをしない限り、Apache-2.0 ライセンスで定義される「あなたが意図的に提出したコントリビューション」は上記のとおりデュアルライセンスされるものとし、追加の条項は一切課されない。
