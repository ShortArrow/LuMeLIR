# PRD: LuMeLIR (Lua Multi-Level Intermediate Representation)

## 1. 製品概要

LuMeLIR は、Lua言語を MLIR (Multi-Level Intermediate Representation) を通じてコンパイルし、CPU、GPU、FPGA、マイコンなど、多様なターゲットでネイティブ実行可能にする次世代コンパイラ・ツールチェーンである。

## 2. ターゲットユーザー

- **組み込み/FPGAエンジニア**: Luaの書きやすさでハードウェア制御ロジックを書きたい層。
- **システムプログラマー**: Rust/C++製のアプリケーションに、ネイティブ性能のスクリプト環境を組み込みたい層。
- **高性能計算 (HPC) 開発者**: Luaで記述した数理モデルをGPUや特殊アクセラレータで回したい層。

## 3. 解決する課題

- **LuaJITの保守性と拡張性の限界**: 職人芸的なアセンブリで構成されるLuaJITに対し、MLIRのモジュール性を利用して現代的な最適化パスを再利用可能にする。
- **実行環境の制約**: 標準Luaが苦手とする「ベアメタル組み込み」や「ヘテロジニアス計算（GPU/FPGA）」への対応。
- **AOTコンパイルの欠如**: スクリプトを事前に完全にバイナリ化し、ランタイムなしで動作させる需要への対応。

## 4. 主要機能 (Features)

### 4.1 LuMeLIR Dialect の定義

Lua独自の動的セマンティクス（Table、Metatable、Upvalue、Coroutine）を抽象化したMLIR方言を定義する。
`lumelir.table_set`, `lumelir.call_meta` などのオペレーションの実装。

### 4.2 段階的 Lowering (段階的変換)

- **High-Level**: Luaの動的な挙動を保持した最適化（定数畳み込み、デッドコード削除）。
- **Mid-Level**: Affine ダイアレクト等を用いた、ループや配列操作の最適化。
- **Low-Level**: LLVM Dialect へ変換し、最終的なバイナリを出力。

### 4.3 マルチターゲット・バックエンド

- **CPU**: x86_64, Arm, RISC-V へのAOTバイナリ生成。
- **MCU**: STM32等のマイコン向けに、標準ライブラリを最小化したベアメタル実行。
- **Acceleration**: クリティカルな計算パスをGPU(SPIR-V)やFPGA(ハードウェア記述)へオフロードする実験的パス。

### 4.4 Rustベースのツールチェーン

- **コンパイラ本体**: Rust + Melior (MLIR binding) による実装。
- **CLI**: `lumelir compile`, `lumelir run` 形式のモダンなインターフェース。
  ADR 0090 で追加された `--emit <stage>` フラグでパイプライン途中
  成果物 (HIR / MLIR text / LLVM IR text) を inspect 可能 (例:
  `lumelir compile hello.lua --emit mlir` は MLIR テキストを stdout に
  書き出す、`--emit llvm -o app.ll` は LLVM IR をファイルへ)。
  `--emit` なしの既存挙動 (native binary 生成) は不変。

## 5. 技術スタック

| カテゴリ | 選定技術 |
| --- | --- |
| 開発言語 | Rust |
| 中間表現基盤 | MLIR (LLVMプロジェクト) |
| パース | Rust製Luaパーサー (nom等) |
| ターゲット | LLVM (CPU), SPIR-V (GPU/Compute) |

## 6. ロードマップ (Milestones)

### Phase 1: Proof of Concept (PoC)

- 数値演算と基本的な関数呼び出しをMLIR経由でAOTコンパイルし、実行ファイルを出力できること。
- `print(1 + 2)` がネイティブバイナリとして動作する。

### Phase 2: Core Semantics

- Luaの肝である「Table」の実装。
- GC（ガベージコレクション）戦略の策定（静的解析による寿命管理、または軽量ランタイムのリンク）。

### Phase 3: Domain Specific Features

- **Rust-Lua Bridge**: Rustの関数をMLIRレベルでインライン化して呼び出す機能。
- 組み込み用レジスタ操作方言の統合。

## 7. 成功指標

- **パフォーマンス**: 単純計算においてLuaJITと同等以上の速度を出す。
- **バイナリサイズ**: 組み込み用途に耐えうる数KB〜数百KB程度のフットプリント。

---

## メモ

このPRDのキモは、**「Luaを単なるスクリプト言語としてではなく、MLIRという強力な変換エンジンへのフロントエンドとして定義し直す」** 点にある。

特に Phase 3 の「Rustとのインライン統合」ができれば、普段書いている Rust プロジェクトに、オーバーヘッドほぼゼロで Lua の柔軟性を持ち込めるようになる。

<!-- Source of Truth. English translation: docs/PRD.md -->
