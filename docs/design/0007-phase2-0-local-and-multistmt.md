# 0007. Phase 2.0: `local` Bindings, Multi-Statement Programs, HIR Introduction

- **Status:** Proposed
- **Date:** 2026-04-26
- **Deciders:** ShortArrow

## Context

Phase 1 (ADR 0006) compiles a single expression statement: `print(1 + 2)`.
The AST is a flat `Expr`; codegen consumes it directly with no name-resolution
or scope layer.

Phase 2.0 is the smallest extension that forces real semantic analysis:

```lua
local x = 1
print(x + 2)
```

This requires (a) statement-level syntax, (b) variable binding & lookup,
(c) sequential codegen across multiple statements. Once these exist, every
later Phase 2 feature (assignment, more locals, control flow, functions)
is incremental rather than structural.

## Decision

### 1. AST: introduce `Stmt`, parse a `Chunk` (statement list)

Add a `Stmt` enum alongside `Expr`. Public entry becomes
`parse(src) -> Result<Chunk, ParseError>`, where `Chunk = Vec<Stmt>`.

```rust
pub enum Stmt {
    Local { name: String, value: Expr, span: Span },
    ExprStmt(Expr),
}
```

Statements are separated by whitespace/newlines; `;` is accepted but
optional (matches Lua 5.4 grammar). Phase 2.0 does **not** add `return`,
assignment to existing locals, or block scopes — those land in 2.1+.

### 2. Lexer additions

New token kinds: `Equals`, `Semicolon`, `Keyword(Keyword)` where
`Keyword` is a small enum starting with `Local`. Identifiers are
post-processed: if the lexeme matches a keyword spelling, the lexer
emits `Keyword` instead of `Ident`. This keeps the parser branch-free
on identifier-vs-keyword discrimination.

### 3. Introduce HIR layer (`src/hir/`)

The AST stays syntactic (names as strings). HIR resolves names to
`LocalId` indices and is the input to codegen.

```rust
pub struct HirChunk { pub locals: Vec<LocalInfo>, pub stmts: Vec<HirStmt> }
pub enum HirStmt { LocalInit { id: LocalId, value: HirExpr }, ExprStmt(HirExpr) }
pub enum HirExpr { Number(f64), Local(LocalId), Call { ... }, BinOp { ... } }
```

`lower_chunk(&Chunk) -> Result<HirChunk, HirError>` walks the AST once,
threading a flat `HashMap<String, LocalId>` for the (currently single)
scope. Errors: `UndefinedName`, `RedefinedLocal` (Lua actually allows
shadowing — Phase 2.0 forbids it temporarily; relaxed in 2.1 with proper
scopes).

**Why a separate HIR now and not later:** the moment `Ident` becomes
ambiguous (could be `print` builtin, undefined name, or a local), name
resolution must happen *somewhere*. Doing it in the parser couples
syntax and semantics; doing it in codegen blocks every codegen test
on having a name table. A minimal HIR is the cheapest separation.

### 4. Codegen: stack slots via `llvm.alloca`

Each `LocalId` gets an `f64` stack slot at function entry, using LLVM
dialect directly:

```mlir
%c1_i32 = llvm.mlir.constant(1 : i32) : i32
%x_slot = llvm.alloca %c1_i32 x f64 : (i32) -> !llvm.ptr
%v = arith.constant 1.0 : f64
llvm.store %v, %x_slot : f64, !llvm.ptr
...
%x_val = llvm.load %x_slot : !llvm.ptr -> f64
```

Phase 1 codegen already operates in LLVM dialect for `printf` and the
format-string global, so staying in LLVM dialect for locals avoids an
extra `--convert-memref-to-llvm` pass and keeps the lowering pipeline
unchanged.

**Alternatives considered:**
- *`memref.alloca` + `--convert-memref-to-llvm` pass.* Rejected — adds
  a pass for no semantic benefit when target lowering is identical.
- *SSA values without alloca.* Rejected — 2.1 adds reassignment, and
  rewriting between 2.0 and 2.1 costs more than carrying alloca through.

### 5. CLI / runtime unchanged

`compile` still takes one source file → one binary. The runtime entry
remains `main` returning `i32`. `print` continues to lower via libc
`printf("%g\n", ...)`.

## Alternatives Considered

- **Skip HIR, resolve names in codegen.** Rejected — couples symbol
  table with MLIR builder state and blocks unit-testing name resolution.
- **AST = HIR (single tree, mutate in place).** Rejected — destroys
  the syntactic AST's value for diagnostics and future LSP work.
- **Use `llvm.alloca` directly, skip `memref`.** Rejected for 2.0 —
  `memref` is the conventional MLIR-level abstraction and adds one pass,
  not a new dialect family.

## Consequences

- Parser surface grows: `parse` returns `Chunk`, not `Expr`. Callers
  (codegen, tests) update accordingly.
- New module `src/hir/` with its own `HirError` (thiserror).
- `src/codegen/` consumes `HirChunk`, not `Expr`. Phase 1 emit logic
  is reused for `BinOp`/`Number`/`Call` after a thin AST→HIR shim.
- Lowering pipeline unchanged from Phase 1.
- Test count grows: lexer +3, parser +5, hir +8, codegen +2, integration +1.

## Out of Scope (deferred)

- Reassignment (`x = 2` after `local x = 1`) → Phase 2.1
- Block scopes / `do ... end` → Phase 2.1
- Additional operators (`-`, `*`, `/`, comparisons) → Phase 2.2
- `if` / `while` → Phase 2.3
- Function definitions → Phase 2.4
- Tables → Phase 2.5+
