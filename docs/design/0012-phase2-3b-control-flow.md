# 0012. Phase 2.3b: `if`/`elseif`/`else` + `while` + Truthiness via `scf`

- **Status:** Accepted
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.3a (ADR 0011) finished the static value model `{ Number, Bool, Nil }`
and unblocked `local b = true`, `print(nil)`, and Lua-conformant
heterogeneous `==`. The next step is the smallest extension that makes
Lua's truthiness rule observable at runtime: control flow.

Without `if` and `while`, neither `nil` falsiness nor "0 is truthy" can
be exercised end-to-end — they're invisible behind `print`'s static
dispatch on `ValueKind`. Phase 2.3b introduces both constructs together
because they share:

1. The same `emit_truthiness(value, kind) -> i1` helper (the new
   piece — Lua: only `nil`/`false` are falsy).
2. The same MLIR `scf` dialect pattern (Region + Block + `scf.yield`).
3. The same lifetime concerns around referencing main-block `slots`
   from inner regions.

`and`/`or`/`not` short-circuit operators are deliberately deferred to
Phase 2.3c so this ADR can focus on the structural codegen change.

## Decision

### 1. Lexer: 5 new keywords

`Keyword` gains `If`, `Then`, `Else`, `Elseif`, `While`. Standard
post-processing path — identifier scan, then `Keyword::from_lexeme`.

### 2. AST and HIR: two new statement kinds

```rust
pub enum StmtKind {
    ...,
    If {
        cond: Expr,
        then_body: Chunk,
        elifs: Vec<(Expr, Chunk)>,
        else_body: Option<Chunk>,
    },
    While { cond: Expr, body: Chunk },
}
```

`HirStmtKind` mirrors the shape with `HirExpr` and `Vec<HirStmt>` in
place of `Expr` and `Chunk`.

The `elifs` vector keeps the elseif chain explicit instead of
desugaring to nested `If` at parse time. This preserves source
structure for diagnostics and lets codegen lower each elseif to a
nested `scf.if` deterministically.

### 3. HIR lowering: scope push/pop per body

`if`/`elseif`/`else`/`while` bodies are independent lexical scopes,
just like `do ... end`. Lowering pushes a fresh scope before each
body and pops on exit. The existing `Vec<HashMap<String, LocalId>>`
scope stack from ADR 0008 carries this without changes — bodies are
just sequences of `lower_stmt` calls inside `scopes.push`/`pop`.

No new `HirError`. Conditions accept any `ValueKind`; truthiness is a
codegen-time concern.

### 4. Codegen: `scf` dialect, two new ops, one helper

#### `emit_truthiness(value, kind) -> Value<i1>`

| `ValueKind` | Output                    |
| ----------- | ------------------------- |
| `Number`    | `arith.constant true : i1` (compile-time fold — Lua: only `nil`/`false` are falsy) |
| `Bool`      | `value` directly (already `i1`) |
| `Nil`       | `arith.constant false : i1` |

#### `emit_if(...)`

Lowers `If { cond, then_body, elifs, else_body }`:

1. Emit `cond` in the parent block; truthify with `emit_truthiness`.
2. Build a `then_region` containing one Block. Emit `then_body` into
   it; terminate with `scf.yield` (no operands).
3. Build an `else_region`. Its content depends on remaining elifs:
   - If `elifs` is empty and `else_body` is `Some(b)`: emit `b` and
     `scf.yield`.
   - If `elifs` is empty and `else_body` is `None`: emit only
     `scf.yield` (empty else).
   - If `elifs` is non-empty: emit a single nested `scf.if` for the
     first elif, with its else region recursively carrying the rest
     of the chain. End with `scf.yield`.
4. Append `scf::r#if(cond_i1, &[], then_region, else_region, loc)` to
   the parent block. Result types are empty — `if` is a statement.

#### `emit_while(...)`

Lowers `While { cond, body }`:

1. Build a `before_region`: emit `cond`, truthify, terminate with
   `scf.condition cond_i1` (no loop-carried values).
2. Build an `after_region`: emit `body`, terminate with `scf.yield`
   (no loop-carried values).
3. Append `scf::r#while(&[], &[], before_region, after_region, loc)`
   to the parent block.

### 5. Allocas remain hoisted at function entry

Phase 2.0/2.1's invariant — every local's stack slot is allocated in
`main`'s entry block — is preserved. Inner regions only `llvm.load` /
`llvm.store` against those slot pointers; the pointer Values dominate
all inner regions. This avoids two problems:

- Lifetime: alloca'd pointers live for `main`'s entire duration, so
  they are valid across any region.
- Verifier: keeping allocas in the entry block matches LLVM's
  `mem2reg` precondition for later optimization passes.

## Alternatives Considered

- **`cf.cond_br` / `cf.br`**. Lower-level: requires explicit successor
  blocks and PHI nodes for SSA join. Rejected — `scf` is the obvious
  fit for structured Lua syntax.
- **`if` as expression (Rust-style yielding values)**. Diverges from
  Lua, which has no expression-form `if`. Rejected.
- **`elseif` as parser desugaring (nested `If` in `else_body`)**. The
  AST loses the source-level chain; diagnostics and codegen have to
  reconstruct it. Rejected.
- **Truthiness as a runtime function call**. Static dispatch on
  `ValueKind` is constant-time at compile time; a runtime call adds
  a function frame for no benefit until we get dynamic types
  (Phase 2.4+).

## Consequences

- `Keyword` +5; `StmtKind` +2; `HirStmtKind` +2.
- `scf` dialect joins `arith`, `func`, `llvm` in the codegen import set.
- `emit_truthiness`, `emit_if`, `emit_while` join the codegen helpers.
- IR structure goes from "flat single block" to "Block with embedded
  Regions". Verifier passes thanks to slot pointers being defined in
  the entry block (dominance preserved).
- Phase 2.3c can implement `and a b` as `scf.if truthiness(a) { a }
  else { b }` (in expression form, using `scf.if`'s value-yielding
  variant) — `emit_truthiness` is reusable.

## Out of Scope (still deferred)

- `and` / `or` / `not` → Phase 2.3c
- `for` numeric / generic loops → Phase 2.3d
- `break` / `continue` / `goto` → Phase 2.4+
- `return` / function definitions → Phase 2.4+
- `repeat ... until` → Phase 2.3d or later
