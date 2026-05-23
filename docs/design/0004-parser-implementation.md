# 0004. Parser Implementation: Hand-Written Recursive Descent with Pratt

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-04-19
- **Deciders:** ShortArrow

## Context

Phase 1 PoC (`print(1 + 2)` → native binary) is past the lexer stage (ADR 0001). The next layer converts a `Token` stream into an AST suitable for lowering to HIR/MIR and eventually MLIR. Requirements for Phase 1:

- Accept the minimal Lua subset `Number`, `Ident`, `Call`, `BinOp::Add`.
- Preserve source spans end-to-end so MLIR diagnostics can point at original byte offsets.
- Be straightforward to extend as Lua 5.4 grammar lands piece by piece (more binary operators, unary operators, string/table literals, statements, control flow).
- Stay consistent with the layering rules (AGENTS.md §4.2): `parser` depends on `lexer` only.
- Report errors with typed variants (ADR 0003: `thiserror`), including a `#[from] LexError` passthrough.

## Decision

**Write the parser by hand using recursive descent, with Pratt parsing for binary operators.** The AST lives inside the `parser` module for now.

### Shape

```rust
// Public API at src/parser/mod.rs
pub fn parse(src: &str) -> Result<Expr, ParseError>;
```

Internally the parser is a small struct holding a `Vec<Token>` (produced by `crate::lexer::lex`) and a cursor index. Parsing proceeds as:

```text
parse(src)
  ↳ lex(src)?
  ↳ parse_expr(MIN_PRECEDENCE)
       ↳ parse_primary()           # Number | Ident | '(' expr ')'
       ↳ parse_call_suffix(primary) # while next token is '(': consume call
       ↳ while next op has prec ≥ current:
             parse_binop_rhs(prec + 1)
  ↳ expect TokenKind::Eof
```

### AST design

```rust
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

pub enum ExprKind {
    Number(f64),
    Ident(String),
    Call { callee: Box<Expr>, args: Vec<Expr> },
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
}

pub enum BinOp { Add }
```

- `Span` is re-used from `lexer::Span` (byte offsets).
- `BinOp` is its own small enum to grow naturally when `*`, `..`, `^`, comparisons join it.
- `Expr { kind, span }` separation keeps enum variants free of repetitive `span` fields.

### Pratt precedence

Phase 1 has one operator (`+`), so the precedence function is a single `match`:

```rust
fn infix_precedence(op: &TokenKind) -> Option<u8> {
    match op {
        TokenKind::Plus => Some(10),
        _ => None,
    }
}
```

New operators add one arm each. Associativity will be introduced as an enum when right-associative operators (`^`, `..`) arrive; hard-coding left-associativity now is fine.

### Error type

```rust
// src/parser/error.rs
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] crate::lexer::LexError),
    #[error("unexpected end of input at byte {offset}")]
    UnexpectedEof { offset: usize },
    #[error("unexpected token {actual:?} at byte {offset}")]
    UnexpectedToken { actual: TokenKind, offset: usize },
}
```

## Alternatives Considered

### `chumsky`
Rejected for Phase 1. `chumsky`'s error recovery story is genuinely useful, but for an AOT compiler's first pass we want fatal errors, not speculative recovery. Its combinator style also forces a particular structure on the AST (often `Rich<Char, Simple<..>>` wrappers) that would cost us more than hand-written recursive descent right now. Revisit when error recovery for IDE-grade tooling becomes a goal.

### `lalrpop`
Rejected. Generated code is large, debugging is indirect, and build-time cost is non-trivial. The expressive power is overkill for Lua's LL(1)-friendly grammar; hand-written recursive descent handles the whole language comfortably.

### Placing AST at top-level `src/ast/`
Rejected for now. Only `parser` produces AST today. Elevating AST to its own module before a second producer (or a pass that transforms AST → AST) exists is premature abstraction. Revisit if/when HIR lowering introduces AST-visiting passes that want to live outside `parser`.

### Pratt vs climbing vs precedence table
Accepted Pratt because it cleanly generalizes to prefix and postfix operators we'll add later (unary `-` `not` `#`, method calls, indexing). The alternatives (precedence climbing, explicit precedence tables) devolve into Pratt once suffix operators enter the picture.

## Consequences

**Positive**
- Zero new crate dependencies.
- Adding a new binary operator is a two-line change: one in the lexer token table (when the operator isn't just `Plus`), one in `infix_precedence`.
- Error types are structured and testable with `assert_eq!`.
- Span composition is local: `BinOp` span = `lhs.span.start..rhs.span.end`, `Call` span = `callee.span.start..rparen.span.end`.

**Negative**
- No automatic error recovery. Acceptable for Phase 1; revisit when a language server story begins.
- Hand-written parsers grow faster than grammar files. Mitigated by test-first development (AGENTS.md §4.3) and by staying strictly on the Phase 1 subset.

**Locked in until superseded**
- If/when a second AST consumer appears (desugar, linter, formatter), move `ast.rs` to `src/ast/` and make `parser` depend on it. Record that as a follow-up ADR.
- Right-associative operators (`^`, `..`) will require extending the precedence function to return `(u8, Assoc)` — not a breaking change to callers.
