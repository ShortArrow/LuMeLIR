# 0035. Phase 2.4b: `repeat ... until cond` Loop

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.3b shipped `while`, Phase 2.3d added the numeric `for`,
and Phase 2.4 wired `break`. The remaining Lua loop form is
`repeat body until cond` — the do-while sibling that runs the
body unconditionally, then evaluates the cond at the bottom and
loops if the cond is **false**.

Two semantic wrinkles distinguish `repeat ... until` from
`while ... do ... end`:

1. **Body always runs at least once.** The cond is a post-test.
2. **Cond can see body-local declarations.** Lua 5.4 §3.3.4 makes
   the until-cond sit inside the body's scope, so
   `repeat local x = compute() until x == sentinel` is a valid
   pattern that needs no extra scope wrapping.

This phase adds the loop form, reuses the existing `_broken`
flag machinery for `break`, and extracts a small shared helper
that the existing `While` and `ForNumeric` paths now share too.

## Decision

### 1. Lexer + parser additions

Two new keywords: `repeat`, `until`. Both lex through the
existing `Keyword::from_lexeme` table. The parser dispatches on
`Keyword::Repeat` from `parse_stmt` to a new `parse_repeat`:

```text
repeat <chunk> until <expr>
```

`parse_chunk_until` already handles open-ended terminator lists
— passing `[Keyword::Until, Eof]` reuses the standard chunk
loop. AST adds `StmtKind::Repeat { body, cond }`.

### 2. HIR shape: `HirStmtKind::Repeat`

Mirrors the `While` shape with the field order swapped to match
the source-level structure:

```rust
HirStmtKind::Repeat {
    body: Vec<HirStmt>,
    cond: HirExpr,
    break_id: Option<LocalId>,
}
```

`break_id` follows the existing `_broken` discipline (Phase 2.4,
ADR 0015): allocated only when `body_contains_break(body)`
returns true.

### 3. HIR lowering: cond inside body's scope

`lower_stmt::Repeat` opens a single lexical scope, lowers the
body statements, then lowers the cond before popping. That keeps
body-local declarations live for the cond expression resolver:

```rust
self.scopes.push(HashMap::new());
let body_hir = self.lower_stmts_maybe_guarded(body)?;
let cond_hir = self.lower_expr(cond)?;
self.scopes.pop();
```

The break flag is allocated **before** the scope push so it
lives in the surrounding scope and can be initialised by the
outer `wrap_with_break_init` helper.

### 4. Codegen: body in `before` region

`emit_repeat` emits an `scf.while` whose `before` region runs
the body, evaluates the cond, inverts it (we continue while
`not cond`), AND-extends with `not _broken` when applicable, and
issues `scf.condition`. The `after` region is empty save for the
required `scf.yield`:

```text
scf.while ()() {
  // before: body, cond, !cond [AND !broken], scf.condition
  body...
  cond_i1   = truthiness(cond)
  not_cond  = cond_i1 XOR true
  continue  = not_cond [AND not_broken]
  scf.condition(continue)
}, {
  // after: empty, no carried values
  scf.yield
}
```

Loading the body in `before` is what gives `repeat ... until`
its do-while shape: `scf.while`'s control transfer hits `before`
once before the first condition test, so the body always runs
at least once.

### 5. Tidy-as-you-go: `wrap_with_break_init` helper

During Green, three loop kinds (`While`, `ForNumeric`, the new
`Repeat`) all needed the same structure:

> If `break_id` is `Some`, wrap the loop in a `Block` with a
> preceding `LocalInit` of the `_broken` flag to `false`.
> Otherwise, return the loop statement unchanged.

The duplicated pattern was lifted into a pure helper:

```rust
fn wrap_with_break_init(
    loop_stmt: HirStmt,
    break_id: Option<LocalId>,
    span: Span,
) -> HirStmt;
```

Both `While` and `ForNumeric` were retrofitted to call it. Net
HIR: ~30 lines deleted from the existing loop arms, ~30 lines
added in the helper, plus the new `Repeat` arm. Behaviour
preserved (test count unchanged through the helper extraction;
new tests count up by 13 for the new feature).

### 6. CA invariants preserved

| Layer    | Change                                                        |
|----------|---------------------------------------------------------------|
| Lexer    | Two new keywords (`Repeat`, `Until`)                          |
| Parser   | New `StmtKind::Repeat`; `parse_repeat`; `strip_span_stmt` arm |
| AST      | `StmtKind::Repeat { body, cond }`                             |
| HIR      | `HirStmtKind::Repeat { body, cond, break_id }`; new lower; shared helper |
| Codegen  | New `HirStmtKind::Repeat` arm in `emit_stmt`; new `emit_repeat`; string-pool walker arm |

Layer dependencies remain `codegen → hir → parser → lexer`. No
inter-layer leaks.

## TDD Process

1. **Step 1 — Tidy First (review only).** The existing loop
   helpers (`and_not_broken`, `wrap_with_broken_guard`,
   `body_contains_break`) were already pure and well-factored —
   no behaviour-preserving refactor was warranted at this point.
2. **Step 2 — Red.** 2 lexer + 4 HIR + 7 e2e tests added,
   referencing not-yet-existent `Keyword::Repeat`/`Until`,
   `StmtKind::Repeat`, `HirStmtKind::Repeat`. `cargo build`
   refused.
3. **Step 3 — Green.** Lexer keywords, parser `parse_repeat`,
   HIR variant + lowering, codegen `emit_repeat`. Tests passed.
4. **Step 4 — Refactor.** During Green the duplicated
   break-init wrap was identified; the
   `wrap_with_break_init` helper was extracted and applied to
   `While`/`ForNumeric`/`Repeat` consistently. `cargo test`
   confirmed no regressions.

## Alternatives Considered

- **Desugar `repeat ... until cond` to `while not cond do
  body end`** at the AST → HIR boundary. Rejected on two
  grounds:
    1. **Semantic mismatch**: `while` evaluates cond before the
       first body execution; `repeat` always runs at least once.
       A pure desugar can't capture that without an explicit
       first-iteration unrolling.
    2. **Scope mismatch**: Lua 5.4's spec gives the until-cond
       access to body-local declarations. A `while`-shaped
       desugar would need to lift those declarations out, which
       changes their source-level lifetime.
- **Encode `repeat` via `scf.execute_region` + a manual
  back-edge.** scf.while is the natural lowering — same shape
  as `while`/`for`, just with body content shifted into the
  `before` region. Rejected the alternative as gratuitous.
- **Treat `until` as a loose statement terminator** rather than
  a full keyword. The reuse-`parse_chunk_until` design puts
  `until` exactly where the parser already knows how to stop.
  No change needed.

## Consequences

- Two new keyword tokens; one new AST variant; one new HIR
  variant; one new codegen helper.
- Shared helper `wrap_with_break_init` reduces total HIR loop
  code by ~25 lines while improving consistency.
- Two lexer unit tests, four HIR unit tests, and seven
  integration tests cover the basic body-runs-once contract,
  cond-sees-body-local, break interactions (basic + nested),
  use inside a user function, and an external-local cond.

## Out of Scope

- **`continue` / `redo`** — not in Lua's grammar.
- **Labelled break / `goto label`** — Lua 5.2+ has `goto`; we
  defer.
- **Unstructured early exit from nested loops** beyond the
  innermost. The existing `_broken` flag pattern is per-loop;
  multi-level break would need labels.
