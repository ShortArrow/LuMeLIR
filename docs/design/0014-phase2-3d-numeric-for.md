# 0014. Phase 2.3d: Numeric `for` Loops via `scf.while` Desugar

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-04-30
- **Deciders:** ShortArrow

## Context

Phase 2.3c (ADR 0013) closed out the logical operators. The last
common Lua control-flow construct missing before functions and
tables is the **numeric `for`** loop:

```lua
for i = 1, 3 do print(i) end                -- 1 2 3
for i = 10, 1, -2 do print(i) end           -- 10 8 6 4 2
```

Generic `for k,v in pairs(t)` requires tables and is deferred to
Phase 2.5+. Phase 2.3d delivers numeric `for` only.

The work fits inside the infrastructure already built:
- `scf.while` from ADR 0012 covers the runtime loop shape.
- `scf.if` expression form from ADR 0013 covers the runtime sign
  dispatch on `step`.
- Per-slot type tracking (ADR 0011) accommodates the loop variable
  and the auxiliary `stop`/`step` slots.

No new MLIR dialect is needed.

## Decision

### 1. Lexer

`TokenKind::Comma` (single character `,`) and `Keyword::For` are added.
`,` is needed only by `for` for now; `keyword_from_lexeme` matches
`"for"` via the existing post-processing path.

### 2. AST and HIR

```rust
pub enum StmtKind {
    ...,
    ForNumeric {
        var: String,
        start: Expr,
        stop: Expr,
        step: Option<Expr>,   // None == implicit `1`
        body: Chunk,
    },
}

pub enum HirStmtKind {
    ...,
    ForNumeric {
        var_id: LocalId,
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,        // synthesised Number(1.0) if Option was None
        body: Vec<HirStmt>,
    },
}
```

`HirExpr` for the implicit step is synthesised at HIR-time with a
zero-width span — codegen never sees `Option`.

### 3. HIR rules

- `start`, `stop`, `step` must each be of `ValueKind::Number`. Other
  kinds (`Bool`, `Nil`) are `HirError::TypeMismatch`.
- The loop body is a fresh lexical scope (`lower_scoped_body` from
  ADR 0008/0012). The loop variable `var` is `declare_local`'d at
  scope entry with `ValueKind::Number` and disappears when the
  scope pops.
- The loop variable is **read-only inside the body** per Lua 5.4
  §3.3.5. Implementation: `LowerCtx` gains
  `readonly_locals: HashSet<LocalId>`; `lower_stmt::Assign` checks
  it and emits `HirError::ReadOnlyAssign { name, offset }` on
  violation. The set is restored after the body lowers.

### 4. Codegen — `scf.while` desugar

`scf.for` requires `index`-typed bounds, but we standardise on `f64`
for numeric values. We instead lower numeric `for` to `scf.while`:

```mlir
// Init in the parent block:
%start_v = <start>
store %start_v -> %var_slot
%stop_v  = <stop>
store %stop_v  -> %stop_slot
%step_v  = <step>
store %step_v  -> %step_slot

scf.while () : () -> () {
  // before region — condition
  %i      = load %var_slot  : f64
  %stop_v = load %stop_slot : f64
  %step_v = load %step_slot : f64
  %pos    = arith.cmpf ogt, %step_v, %zero : i1
  %cond   = scf.if %pos -> (i1) {
    %le = arith.cmpf ole, %i, %stop_v : i1
    scf.yield %le : i1
  } else {
    %ge = arith.cmpf oge, %i, %stop_v : i1
    scf.yield %ge : i1
  }
  scf.condition(%cond)
} do {
  // after region — body + step
  <body>
  %i_now  = load %var_slot
  %step_v = load %step_slot
  %i_next = arith.addf %i_now, %step_v : f64
  store %i_next -> %var_slot
  scf.yield
}
```

`stop` and `step` are evaluated **once** before the loop (Lua 5.4
§3.3.5) and stored in dedicated stack slots. Sign dispatch is
runtime so that variable `step` works:

```lua
local s = -2
for i = 10, 1, s do ... end
```

LLVM constant-folds the dispatch when `step` is a literal.

`emit_for_numeric` allocates three additional `f64` slots (`var`,
`stop`, `step`) at function entry by extending the existing
slot-hoisting pass — actually, only `var` is in `chunk.locals`; the
auxiliary `stop`/`step` slots are allocated locally inside
`emit_for_numeric` against the parent block (still in `main`'s
entry, since `for` only occurs inside `main` in the current phase).

### 5. CLI / runtime unchanged

No new libm symbols. The `printf("%g\n", ...)` path renders loop
values exactly like other numbers.

## Alternatives Considered

- **`scf.for`**. Native MLIR loop, but bounds and induction variable
  are `index`-typed (`i64`-equivalent). Would require f64 ↔ index
  conversion at every boundary and break uniformity with the rest
  of the value model. Rejected.
- **Compile-time only `step` (literal restriction).** Would let the
  sign be baked into a fixed `cmpf` predicate. Rejected — drops the
  variable-step idiom and adds little simplicity.
- **Re-evaluate `start`/`stop`/`step` per iteration.** Lua-spec
  violation. Rejected.
- **Loop variable read-only as warning, not error.** The cost of
  enforcement is a `HashSet` lookup; matching Lua's spec is the
  right default.

## Consequences

- `Keyword` +1 (`For`), `TokenKind` +1 (`Comma`).
- `StmtKind` +1, `HirStmtKind` +1.
- `HirError` +1 (`ReadOnlyAssign`).
- `LowerCtx` gains `readonly_locals: HashSet<LocalId>`.
- New `emit_for_numeric` codegen helper; `scf.if` expression form
  used a second time (after `and`/`or`).
- Auxiliary `stop`/`step` allocas added in the parent block on
  demand. Phase 2.4 (functions) will revisit slot allocation if
  nested function scopes reshape this.

## Out of Scope (still deferred)

- Generic `for k,v in pairs(t)` → Phase 2.5+ (tables required).
- `break` / `continue` / `goto` → Phase 2.4+.
- `repeat ... until` → Phase 2.3e or later.
- Numeric string coercion (`"1"` as number) → Phase 2.5+.
- Static `step == 0` detection → later phase.
- Function definitions / `return` → Phase 2.4.
