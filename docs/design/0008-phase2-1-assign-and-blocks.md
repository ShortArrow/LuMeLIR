# 0008. Phase 2.1: Reassignment and `do ... end` Block Scopes

- **Status:** Accepted
- **Date:** 2026-04-29
- **Deciders:** ShortArrow

## Context

Phase 2.0 (ADR 0007) added `local` bindings, multi-statement chunks, and an
HIR layer with a single flat scope. Three rough edges remain before broader
control flow is worth attempting:

1. Locals are write-once. `x = 2` after `local x = 1` is rejected.
2. Re-`local`ing the same name in the same scope is rejected, but real Lua
   allows shadowing.
3. There is no nested scope construct, so name resolution is trivially flat.

Phase 2.1 is the smallest extension that fixes all three at once:

```lua
local x = 1
x = 2
do
  local x = 99
  print(x)    -- 99
end
print(x)      -- 2
```

This forces a real scope stack in the HIR, exercises the existing alloca
slot model under reassignment, and unlocks `if` / `while` (Phase 2.3) which
need block scopes anyway.

## Decision

### 1. Lexer: two new keywords

Extend `Keyword` with `Do` and `End`. The keyword classifier already
post-processes identifiers; this is a one-line addition per keyword.

### 2. Parser: two new statement kinds

```rust
pub enum StmtKind {
    Local { name: String, value: Expr },
    Assign { name: String, value: Expr },   // NEW
    Block(Chunk),                            // NEW (do ... end)
    ExprStmt(Expr),
}
```

Disambiguation in `parse_stmt`:

| First token              | Statement form              |
| ------------------------ | --------------------------- |
| `Keyword(Local)`         | `Local`                     |
| `Keyword(Do)`            | `Block` (until `end`)       |
| `Ident` followed by `=`  | `Assign`                    |
| else                     | `ExprStmt`                  |

The `Ident` `=` lookahead is one token, easy to express by peeking.
Function calls (`print(x)`) keep working: `print` is `Ident`, next token
is `LParen`, so we fall through to `ExprStmt`.

### 3. HIR: scope stack + new statement kinds

Replace `LowerCtx`'s flat `HashMap<String, LocalId>` with a `Vec` of maps
(one per active lexical scope). Lookup walks the stack from innermost out.
`local` always pushes a fresh `LocalId` and binds it in the current
(top) scope, masking any outer same-name binding. **Shadowing is allowed
in any scope, including the same one** — matching Lua 5.4 semantics
(removes `RedefinedLocal` from `HirError`).

```rust
pub enum HirStmtKind {
    LocalInit { id: LocalId, value: HirExpr },
    Assign    { id: LocalId, value: HirExpr },   // NEW
    Block     { stmts: Vec<HirStmt> },           // NEW
    ExprStmt(HirExpr),
}
```

`Assign` resolves the LHS name to a `LocalId` at lower-time. Assigning
to an undefined name is `HirError::UndefinedName` for Phase 2.1 — Lua
would treat it as a global write, but globals are out of scope until the
table phase.

`locals: Vec<LocalInfo>` stays a flat ledger of every binding ever
introduced in the chunk, regardless of scope. Each gets its own slot at
function entry; scope only gates *which name resolves to which id*.

### 4. Codegen: store for `Assign`, recursion for `Block`

The Phase 2.0 alloca hoisting (one slot per `LocalInfo` at function
entry) already supports everything Phase 2.1 needs:

- `Assign { id, value }` → emit value, then `llvm.store value, slot[id]`.
  Identical operand shape to `LocalInit` after the slot is created.
- `Block { stmts }` → emit each child statement in order. Slots are
  already at function entry, so block boundaries have no codegen
  meaning beyond statement sequencing. Lifetimes are handled by the
  scope stack at HIR-time, not at MLIR-time.

No new dialects, no pipeline change.

### 5. Errors

- `HirError::RedefinedLocal` is **removed**. Shadowing is legal.
- `HirError::UndefinedName` now also covers `x = ...` to an unbound name.
  Same variant, same message shape.

## Alternatives Considered

- **Per-scope alloca placement.** Allocate slots at the start of each
  block instead of hoisting. Rejected — LLVM optimizer hoists them
  anyway, and per-scope alloca complicates the slot-table representation
  for zero observable benefit at this phase.
- **Treat assignment to undefined name as global write.** Rejected —
  globals require the table runtime; emitting them now means dead code
  until Phase 2.5+.
- **Keep `RedefinedLocal` for same-scope only.** Rejected — Lua allows
  same-scope shadowing, so the rule diverges from the language we are
  building. Removing it now is cheaper than a 2.2 follow-up.

## Consequences

- `HirError` shrinks by one variant; downstream `match` arms simplify.
- `LowerCtx` gains a `Vec<HashMap<…>>` and push/pop helpers. ~30 LOC.
- Parser test count: +3 (assign, do/end, nested do).
- HIR test count: +4 (assign resolves, assign-to-undef errors, shadow
  in same scope, shadow in inner scope shadows then restores).
- Codegen test count: +2 (assign emits store, block emits sequenced ops).
- Integration: one new end-to-end test for the snippet at the top of
  this ADR (expected stdout `99\n2\n`).

## Out of Scope (still deferred)

- More operators (`-`, `*`, `/`, comparisons) → Phase 2.2
- `if` / `while` → Phase 2.3
- Function definitions → Phase 2.4
- Tables / globals → Phase 2.5+
