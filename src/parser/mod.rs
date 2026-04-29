//! Parser — turns a [`crate::lexer`] token stream into an AST.
//!
//! Public API is a single pure function [`parse`]. See
//! `docs/design/0004-parser-implementation.md` for the implementation
//! strategy (hand-written recursive descent with Pratt parsing) and
//! `docs/design/0007-phase2-0-local-and-multistmt.md` for the
//! statement-level extensions added in Phase 2.0.

mod ast;
mod error;

pub use ast::{BinOp, Chunk, Expr, ExprKind, Stmt, StmtKind, UnaryOp};
pub use error::ParseError;

use crate::lexer::{self, Keyword, Span, Token, TokenKind};

/// Parse a Lua source string into a [`Chunk`] (statement list).
pub fn parse(src: &str) -> Result<Chunk, ParseError> {
    let tokens = lexer::lex(src)?;
    let mut parser = Parser::new(&tokens);
    let chunk = parser.parse_chunk()?;
    parser.expect_eof()?;
    Ok(chunk)
}

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl<'t> Parser<'t> {
    fn new(tokens: &'t [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &'t Token {
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> &'t Token {
        let tok = &self.tokens[self.pos];
        if !matches!(tok.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        tok
    }

    fn expect_eof(&self) -> Result<(), ParseError> {
        let tok = self.peek();
        match tok.kind {
            TokenKind::Eof => Ok(()),
            _ => Err(ParseError::UnexpectedToken {
                actual: tok.kind.clone(),
                offset: tok.span.start,
            }),
        }
    }

    fn parse_chunk(&mut self) -> Result<Chunk, ParseError> {
        self.parse_chunk_until(&[TokenKind::Eof])
    }

    /// Parse statements until the next significant token matches one of
    /// `terminators`. The terminator itself is **not** consumed.
    fn parse_chunk_until(&mut self, terminators: &[TokenKind]) -> Result<Chunk, ParseError> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::Semicolon) {
                self.bump();
            }
            if terminators.iter().any(|t| t == &self.peek().kind) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek().kind {
            TokenKind::Keyword(Keyword::Local) => self.parse_local(),
            TokenKind::Keyword(Keyword::Do) => self.parse_block(),
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::While) => self.parse_while(),
            TokenKind::Ident(_) if matches!(self.peek_kind_at(1), Some(TokenKind::Equals)) => {
                self.parse_assign()
            }
            _ => {
                let expr = self.parse_expr(0)?;
                let span = expr.span;
                Ok(Stmt::new(StmtKind::ExprStmt(expr), span))
            }
        }
    }

    fn expect_keyword(&mut self, kw: Keyword) -> Result<Token, ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Keyword(k) if k == kw => {
                self.bump();
                Ok(tok)
            }
            TokenKind::Eof => Err(ParseError::UnexpectedEof {
                offset: tok.span.start,
            }),
            other => Err(ParseError::UnexpectedToken {
                actual: other,
                offset: tok.span.start,
            }),
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let if_tok = self.bump().clone(); // 'if'
        let cond = self.parse_expr(0)?;
        self.expect_keyword(Keyword::Then)?;
        let terminators = [
            TokenKind::Keyword(Keyword::Elseif),
            TokenKind::Keyword(Keyword::Else),
            TokenKind::Keyword(Keyword::End),
            TokenKind::Eof,
        ];
        let then_body = self.parse_chunk_until(&terminators)?;

        let mut elifs: Vec<(Expr, Chunk)> = Vec::new();
        while matches!(self.peek().kind, TokenKind::Keyword(Keyword::Elseif)) {
            self.bump();
            let elif_cond = self.parse_expr(0)?;
            self.expect_keyword(Keyword::Then)?;
            let elif_body = self.parse_chunk_until(&terminators)?;
            elifs.push((elif_cond, elif_body));
        }

        let else_body = if matches!(self.peek().kind, TokenKind::Keyword(Keyword::Else)) {
            self.bump();
            let body =
                self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
            Some(body)
        } else {
            None
        };

        let end_tok = self.expect_keyword(Keyword::End)?;
        let span = Span::new(if_tok.span.start, end_tok.span.end);
        Ok(Stmt::new(
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            },
            span,
        ))
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        let while_tok = self.bump().clone();
        let cond = self.parse_expr(0)?;
        self.expect_keyword(Keyword::Do)?;
        let body = self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
        let end_tok = self.expect_keyword(Keyword::End)?;
        let span = Span::new(while_tok.span.start, end_tok.span.end);
        Ok(Stmt::new(StmtKind::While { cond, body }, span))
    }

    fn peek_kind_at(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let name_tok = self.peek().clone();
        let TokenKind::Ident(name) = name_tok.kind else {
            unreachable!("parse_assign requires an Ident at peek()");
        };
        self.bump(); // ident
        self.bump(); // '=' (already verified by lookahead)
        let value = self.parse_expr(0)?;
        let span = Span::new(name_tok.span.start, value.span.end);
        Ok(Stmt::new(StmtKind::Assign { name, value }, span))
    }

    fn parse_block(&mut self) -> Result<Stmt, ParseError> {
        let do_tok = self.bump().clone();
        let body = self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::Keyword(Keyword::End) => {
                self.bump();
                let span = Span::new(do_tok.span.start, end_tok.span.end);
                Ok(Stmt::new(StmtKind::Block(body), span))
            }
            TokenKind::Eof => Err(ParseError::UnexpectedEof {
                offset: end_tok.span.start,
            }),
            other => Err(ParseError::UnexpectedToken {
                actual: other,
                offset: end_tok.span.start,
            }),
        }
    }

    fn parse_local(&mut self) -> Result<Stmt, ParseError> {
        let local_tok = self.bump().clone();
        let name_tok = self.peek().clone();
        let name = match name_tok.kind {
            TokenKind::Ident(ref n) => {
                self.bump();
                n.clone()
            }
            TokenKind::Eof => {
                return Err(ParseError::UnexpectedEof {
                    offset: name_tok.span.start,
                });
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    actual: other,
                    offset: name_tok.span.start,
                });
            }
        };
        let eq_tok = self.peek().clone();
        match eq_tok.kind {
            TokenKind::Equals => {
                self.bump();
            }
            TokenKind::Eof => {
                return Err(ParseError::UnexpectedEof {
                    offset: eq_tok.span.start,
                });
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    actual: other,
                    offset: eq_tok.span.start,
                });
            }
        }
        let value = self.parse_expr(0)?;
        let span = Span::new(local_tok.span.start, value.span.end);
        Ok(Stmt::new(StmtKind::Local { name, value }, span))
    }

    fn parse_expr(&mut self, min_prec: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        lhs = self.parse_call_suffix(lhs)?;

        while let Some(info) = infix_op(&self.peek().kind) {
            if info.prec < min_prec {
                break;
            }
            let op_tok = self.bump();
            let op = binop_from_token(&op_tok.kind).expect("infix_op guarantees a known operator");
            let next_min = if info.right_assoc {
                info.prec
            } else {
                info.prec + 1
            };
            let rhs = self.parse_expr(next_min)?;
            let span = Span::new(lhs.span.start, rhs.span.end);
            lhs = Expr::new(
                ExprKind::BinOp {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }

        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let unary_op = match self.peek().kind {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Keyword(Keyword::Not) => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(op) = unary_op {
            let op_tok = self.bump().clone();
            // Unary operators bind at PREC_UNARY (12) — tighter than
            // `*`/`/`/`%` and `<`/`==`/etc., looser than `^`.
            let operand = self.parse_expr(PREC_UNARY)?;
            let span = Span::new(op_tok.span.start, operand.span.end);
            return Ok(Expr::new(
                ExprKind::UnaryOp {
                    op,
                    operand: Box::new(operand),
                },
                span,
            ));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Number(value) => {
                self.bump();
                Ok(Expr::new(ExprKind::Number(value), tok.span))
            }
            TokenKind::Ident(ref name) => {
                self.bump();
                Ok(Expr::new(ExprKind::Ident(name.clone()), tok.span))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(true), tok.span))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(false), tok.span))
            }
            TokenKind::Keyword(Keyword::Nil) => {
                self.bump();
                Ok(Expr::new(ExprKind::Nil, tok.span))
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_expr(0)?;
                let closing = self.peek().clone();
                match closing.kind {
                    TokenKind::RParen => {
                        self.bump();
                        Ok(inner)
                    }
                    TokenKind::Eof => Err(ParseError::UnexpectedEof {
                        offset: closing.span.start,
                    }),
                    other => Err(ParseError::UnexpectedToken {
                        actual: other,
                        offset: closing.span.start,
                    }),
                }
            }
            TokenKind::Eof => Err(ParseError::UnexpectedEof {
                offset: tok.span.start,
            }),
            other => Err(ParseError::UnexpectedToken {
                actual: other,
                offset: tok.span.start,
            }),
        }
    }

    fn parse_call_suffix(&mut self, mut callee: Expr) -> Result<Expr, ParseError> {
        while matches!(self.peek().kind, TokenKind::LParen) {
            self.bump();
            let mut args = Vec::new();
            if !matches!(self.peek().kind, TokenKind::RParen) {
                args.push(self.parse_expr(0)?);
            }
            let closing = self.peek().clone();
            match closing.kind {
                TokenKind::RParen => {
                    self.bump();
                    let span = Span::new(callee.span.start, closing.span.end);
                    callee = Expr::new(
                        ExprKind::Call {
                            callee: Box::new(callee),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::Eof => {
                    return Err(ParseError::UnexpectedEof {
                        offset: closing.span.start,
                    });
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        actual: other,
                        offset: closing.span.start,
                    });
                }
            }
        }
        Ok(callee)
    }
}

/// Precedence ladder per ADRs 0009, 0010, and 0013 (Lua 5.4 §3.4.8 subset).
const PREC_OR: u8 = 5;
const PREC_AND: u8 = 6;
const PREC_CMP: u8 = 8;
const PREC_ADD: u8 = 10;
const PREC_MUL: u8 = 11;
const PREC_UNARY: u8 = 12;
const PREC_POW: u8 = 13;

struct InfixInfo {
    prec: u8,
    right_assoc: bool,
}

fn infix_op(kind: &TokenKind) -> Option<InfixInfo> {
    match kind {
        TokenKind::Keyword(Keyword::Or) => Some(InfixInfo {
            prec: PREC_OR,
            right_assoc: false,
        }),
        TokenKind::Keyword(Keyword::And) => Some(InfixInfo {
            prec: PREC_AND,
            right_assoc: false,
        }),
        TokenKind::Lt
        | TokenKind::Gt
        | TokenKind::LtEq
        | TokenKind::GtEq
        | TokenKind::EqEq
        | TokenKind::TildeEq => Some(InfixInfo {
            prec: PREC_CMP,
            right_assoc: false,
        }),
        TokenKind::Plus | TokenKind::Minus => Some(InfixInfo {
            prec: PREC_ADD,
            right_assoc: false,
        }),
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Some(InfixInfo {
            prec: PREC_MUL,
            right_assoc: false,
        }),
        TokenKind::Caret => Some(InfixInfo {
            prec: PREC_POW,
            right_assoc: true,
        }),
        _ => None,
    }
}

fn binop_from_token(kind: &TokenKind) -> Option<BinOp> {
    match kind {
        TokenKind::Plus => Some(BinOp::Add),
        TokenKind::Minus => Some(BinOp::Sub),
        TokenKind::Star => Some(BinOp::Mul),
        TokenKind::Slash => Some(BinOp::Div),
        TokenKind::Percent => Some(BinOp::Mod),
        TokenKind::Caret => Some(BinOp::Pow),
        TokenKind::Lt => Some(BinOp::Lt),
        TokenKind::LtEq => Some(BinOp::Le),
        TokenKind::Gt => Some(BinOp::Gt),
        TokenKind::GtEq => Some(BinOp::Ge),
        TokenKind::EqEq => Some(BinOp::Eq),
        TokenKind::TildeEq => Some(BinOp::Ne),
        TokenKind::Keyword(Keyword::And) => Some(BinOp::And),
        TokenKind::Keyword(Keyword::Or) => Some(BinOp::Or),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single_expr(src: &str) -> Result<ExprKind, ParseError> {
        let chunk = parse(src)?;
        assert_eq!(chunk.len(), 1, "expected one statement");
        match chunk.into_iter().next().unwrap().kind {
            StmtKind::ExprStmt(e) => Ok(strip_span_expr(e).kind),
            other => panic!("expected ExprStmt, got {other:?}"),
        }
    }

    fn parse_single_stmt(src: &str) -> Result<Stmt, ParseError> {
        let chunk = parse(src)?;
        assert_eq!(chunk.len(), 1, "expected one statement");
        Ok(chunk.into_iter().next().unwrap())
    }

    fn number(v: f64) -> Expr {
        Expr::new(ExprKind::Number(v), Span::new(0, 0))
    }

    fn ident(name: &str) -> Expr {
        Expr::new(ExprKind::Ident(name.to_owned()), Span::new(0, 0))
    }

    fn strip_span_expr(expr: Expr) -> Expr {
        let kind = match expr.kind {
            ExprKind::Number(v) => ExprKind::Number(v),
            ExprKind::Bool(b) => ExprKind::Bool(b),
            ExprKind::Nil => ExprKind::Nil,
            ExprKind::Ident(n) => ExprKind::Ident(n),
            ExprKind::Call { callee, args } => ExprKind::Call {
                callee: Box::new(strip_span_expr(*callee)),
                args: args.into_iter().map(strip_span_expr).collect(),
            },
            ExprKind::BinOp { op, lhs, rhs } => ExprKind::BinOp {
                op,
                lhs: Box::new(strip_span_expr(*lhs)),
                rhs: Box::new(strip_span_expr(*rhs)),
            },
            ExprKind::UnaryOp { op, operand } => ExprKind::UnaryOp {
                op,
                operand: Box::new(strip_span_expr(*operand)),
            },
        };
        Expr::new(kind, Span::new(0, 0))
    }

    fn strip_span_stmt(stmt: Stmt) -> Stmt {
        let kind = match stmt.kind {
            StmtKind::Local { name, value } => StmtKind::Local {
                name,
                value: strip_span_expr(value),
            },
            StmtKind::Assign { name, value } => StmtKind::Assign {
                name,
                value: strip_span_expr(value),
            },
            StmtKind::Block(body) => {
                StmtKind::Block(body.into_iter().map(strip_span_stmt).collect())
            }
            StmtKind::If {
                cond,
                then_body,
                elifs,
                else_body,
            } => StmtKind::If {
                cond: strip_span_expr(cond),
                then_body: then_body.into_iter().map(strip_span_stmt).collect(),
                elifs: elifs
                    .into_iter()
                    .map(|(c, b)| {
                        (
                            strip_span_expr(c),
                            b.into_iter().map(strip_span_stmt).collect(),
                        )
                    })
                    .collect(),
                else_body: else_body.map(|b| b.into_iter().map(strip_span_stmt).collect()),
            },
            StmtKind::While { cond, body } => StmtKind::While {
                cond: strip_span_expr(cond),
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::ExprStmt(e) => StmtKind::ExprStmt(strip_span_expr(e)),
        };
        Stmt::new(kind, Span::new(0, 0))
    }

    #[test]
    fn parse_integer_literal_yields_number_expr() {
        assert_eq!(parse_single_expr("42").unwrap(), ExprKind::Number(42.0));
    }

    #[test]
    fn parse_identifier_yields_ident_expr() {
        assert_eq!(
            parse_single_expr("print").unwrap(),
            ExprKind::Ident("print".to_owned()),
        );
    }

    #[test]
    fn parse_simple_addition_yields_binop_expr() {
        let kind = parse_single_expr("1 + 2").expect("addition must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Add,
                lhs: Box::new(number(1.0)),
                rhs: Box::new(number(2.0)),
            },
        );
    }

    #[test]
    fn parse_left_associative_chain() {
        let kind = parse_single_expr("1 + 2 + 3").expect("chained addition must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Add,
                lhs: Box::new(Expr::new(
                    ExprKind::BinOp {
                        op: BinOp::Add,
                        lhs: Box::new(number(1.0)),
                        rhs: Box::new(number(2.0)),
                    },
                    Span::new(0, 0),
                )),
                rhs: Box::new(number(3.0)),
            },
        );
    }

    #[test]
    fn parse_call_without_args_yields_call_expr() {
        let kind = parse_single_expr("f()").expect("zero-arg call must parse");
        assert_eq!(
            kind,
            ExprKind::Call {
                callee: Box::new(ident("f")),
                args: vec![],
            },
        );
    }

    #[test]
    fn parse_call_with_single_arg() {
        let kind = parse_single_expr("print(42)").expect("single-arg call must parse");
        assert_eq!(
            kind,
            ExprKind::Call {
                callee: Box::new(ident("print")),
                args: vec![number(42.0)],
            },
        );
    }

    #[test]
    fn parse_print_call_with_addition_matches_phase1_goal() {
        let kind = parse_single_expr("print(1 + 2)").expect("Phase 1 target must parse");
        assert_eq!(
            kind,
            ExprKind::Call {
                callee: Box::new(ident("print")),
                args: vec![Expr::new(
                    ExprKind::BinOp {
                        op: BinOp::Add,
                        lhs: Box::new(number(1.0)),
                        rhs: Box::new(number(2.0)),
                    },
                    Span::new(0, 0),
                )],
            },
        );
    }

    #[test]
    fn parse_call_span_covers_entire_call_site() {
        let src = "print(1 + 2)";
        let stmt = parse_single_stmt(src).expect("Phase 1 target must parse");
        match stmt.kind {
            StmtKind::ExprStmt(e) => {
                assert_eq!(e.span, Span::new(0, src.len()));
                assert!(matches!(e.kind, ExprKind::Call { .. }));
            }
            other => panic!("expected ExprStmt, got {other:?}"),
        }
    }

    #[test]
    fn parse_unexpected_eof_when_rhs_missing() {
        let err = parse("1 +").expect_err("dangling operator must fail");
        assert_eq!(err, ParseError::UnexpectedEof { offset: 3 });
    }

    #[test]
    fn parse_propagates_lex_error() {
        let err = parse("@").expect_err("invalid character must surface as LexError");
        assert!(matches!(err, ParseError::Lex(_)));
    }

    #[test]
    fn parse_two_bare_expressions_yield_two_statements() {
        // Phase 2.0 treats bare expressions as statements; "1 2" is two stmts.
        let chunk = parse("1 2").expect("two expressions parse as two statements");
        assert_eq!(chunk.len(), 2);
    }

    #[test]
    fn parse_local_declaration_yields_local_stmt() {
        let stmt = parse_single_stmt("local x = 1").expect("local decl must parse");
        let stripped = strip_span_stmt(stmt);
        assert_eq!(
            stripped.kind,
            StmtKind::Local {
                name: "x".to_owned(),
                value: number(1.0),
            },
        );
    }

    #[test]
    fn parse_two_statements_yields_two_chunk_entries() {
        let chunk = parse("local x = 1\nprint(x + 2)").expect("two-stmt chunk must parse");
        assert_eq!(chunk.len(), 2);
        let stripped: Vec<_> = chunk.into_iter().map(strip_span_stmt).collect();
        assert_eq!(
            stripped[0].kind,
            StmtKind::Local {
                name: "x".to_owned(),
                value: number(1.0),
            },
        );
        match &stripped[1].kind {
            StmtKind::ExprStmt(e) => {
                assert!(matches!(e.kind, ExprKind::Call { .. }));
            }
            other => panic!("expected ExprStmt, got {other:?}"),
        }
    }

    #[test]
    fn parse_optional_semicolons_are_ignored() {
        let chunk = parse(";;local x = 1; ; print(x);;").expect("semicolons must be optional");
        assert_eq!(chunk.len(), 2);
    }

    #[test]
    fn parse_local_without_initializer_is_rejected() {
        let err = parse("local x").expect_err("Phase 2.0 requires initializer");
        assert_eq!(err, ParseError::UnexpectedEof { offset: 7 });
    }

    #[test]
    fn parse_local_without_name_is_rejected() {
        let err = parse("local = 1").expect_err("missing local name must fail");
        assert!(matches!(err, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn parse_empty_source_yields_empty_chunk() {
        let chunk = parse("").expect("empty source must parse to empty chunk");
        assert!(chunk.is_empty());
    }

    #[test]
    fn parse_assignment_yields_assign_stmt() {
        let stmt = parse_single_stmt("x = 2").expect("assignment must parse");
        let stripped = strip_span_stmt(stmt);
        assert_eq!(
            stripped.kind,
            StmtKind::Assign {
                name: "x".to_owned(),
                value: number(2.0),
            },
        );
    }

    #[test]
    fn parse_empty_do_end_yields_block_with_no_body() {
        let stmt = parse_single_stmt("do end").expect("empty block must parse");
        match stmt.kind {
            StmtKind::Block(body) => assert!(body.is_empty()),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn parse_do_end_with_inner_local_yields_block() {
        let stmt = parse_single_stmt("do local x = 1 end").expect("block with local must parse");
        let stripped = strip_span_stmt(stmt);
        match stripped.kind {
            StmtKind::Block(body) => {
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0].kind, StmtKind::Local { .. }));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn parse_nested_do_end_blocks() {
        let chunk = parse("do do print(1) end end").expect("nested blocks must parse");
        assert_eq!(chunk.len(), 1);
        let StmtKind::Block(outer) = &chunk[0].kind else {
            panic!("expected outer Block");
        };
        assert_eq!(outer.len(), 1);
        let StmtKind::Block(inner) = &outer[0].kind else {
            panic!("expected inner Block");
        };
        assert_eq!(inner.len(), 1);
        assert!(matches!(inner[0].kind, StmtKind::ExprStmt(_)));
    }

    fn binop(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        Expr::new(
            ExprKind::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            Span::new(0, 0),
        )
    }

    fn unary(op: UnaryOp, operand: Expr) -> Expr {
        Expr::new(
            ExprKind::UnaryOp {
                op,
                operand: Box::new(operand),
            },
            Span::new(0, 0),
        )
    }

    #[test]
    fn parse_subtraction_yields_sub_binop() {
        let kind = parse_single_expr("3 - 1").expect("subtraction must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Sub,
                lhs: Box::new(number(3.0)),
                rhs: Box::new(number(1.0)),
            },
        );
    }

    #[test]
    fn parse_multiplication_binds_tighter_than_addition() {
        // 1 + 2 * 3 == 1 + (2 * 3)
        let kind = parse_single_expr("1 + 2 * 3").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Add,
                lhs: Box::new(number(1.0)),
                rhs: Box::new(binop(BinOp::Mul, number(2.0), number(3.0))),
            },
        );
    }

    #[test]
    fn parse_division_and_modulo_share_mul_precedence() {
        // 6 / 2 % 3 == (6 / 2) % 3 (left-assoc within the same level)
        let kind = parse_single_expr("6 / 2 % 3").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Mod,
                lhs: Box::new(binop(BinOp::Div, number(6.0), number(2.0))),
                rhs: Box::new(number(3.0)),
            },
        );
    }

    #[test]
    fn parse_caret_is_right_associative() {
        // 2 ^ 3 ^ 2 == 2 ^ (3 ^ 2)
        let kind = parse_single_expr("2 ^ 3 ^ 2").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Pow,
                lhs: Box::new(number(2.0)),
                rhs: Box::new(binop(BinOp::Pow, number(3.0), number(2.0))),
            },
        );
    }

    #[test]
    fn parse_unary_minus_yields_unary_neg() {
        let kind = parse_single_expr("-5").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(number(5.0)),
            },
        );
    }

    #[test]
    fn parse_unary_minus_binds_looser_than_caret() {
        // -x^2 == -(x^2)
        let kind = parse_single_expr("-2 ^ 3").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(binop(BinOp::Pow, number(2.0), number(3.0))),
            },
        );
    }

    #[test]
    fn parse_unary_minus_binds_tighter_than_mul() {
        // -2 * 3 == (-2) * 3
        let kind = parse_single_expr("-2 * 3").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Mul,
                lhs: Box::new(unary(UnaryOp::Neg, number(2.0))),
                rhs: Box::new(number(3.0)),
            },
        );
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::new(ExprKind::Bool(b), Span::new(0, 0))
    }

    #[test]
    fn parse_true_literal_yields_bool_expr() {
        assert_eq!(parse_single_expr("true").unwrap(), ExprKind::Bool(true));
    }

    #[test]
    fn parse_false_literal_yields_bool_expr() {
        assert_eq!(parse_single_expr("false").unwrap(), ExprKind::Bool(false));
    }

    #[test]
    fn parse_lt_yields_lt_binop() {
        let kind = parse_single_expr("1 < 2").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Lt,
                lhs: Box::new(number(1.0)),
                rhs: Box::new(number(2.0)),
            },
        );
    }

    #[test]
    fn parse_eq_eq_yields_eq_binop() {
        let kind = parse_single_expr("1 == 1").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Eq,
                lhs: Box::new(number(1.0)),
                rhs: Box::new(number(1.0)),
            },
        );
    }

    #[test]
    fn parse_comparison_lower_prec_than_addition() {
        // 1 + 2 < 3 * 4  ==  (1 + 2) < (3 * 4)
        let kind = parse_single_expr("1 + 2 < 3 * 4").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Lt,
                lhs: Box::new(binop(BinOp::Add, number(1.0), number(2.0))),
                rhs: Box::new(binop(BinOp::Mul, number(3.0), number(4.0))),
            },
        );
    }

    #[test]
    fn parse_bool_expr_in_print_call() {
        let kind = parse_single_expr("print(true)").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::Call {
                callee: Box::new(ident("print")),
                args: vec![bool_lit(true)],
            },
        );
    }

    #[test]
    fn parse_nil_literal_yields_nil_expr() {
        assert_eq!(parse_single_expr("nil").unwrap(), ExprKind::Nil);
    }

    #[test]
    fn parse_nil_in_print_call() {
        let kind = parse_single_expr("print(nil)").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::Call {
                callee: Box::new(ident("print")),
                args: vec![Expr::new(ExprKind::Nil, Span::new(0, 0))],
            },
        );
    }

    #[test]
    fn parse_simple_if_yields_if_stmt() {
        let stmt = parse_single_stmt("if 1 then end").expect("must parse");
        match stmt.kind {
            StmtKind::If {
                then_body,
                elifs,
                else_body,
                ..
            } => {
                assert!(then_body.is_empty());
                assert!(elifs.is_empty());
                assert!(else_body.is_none());
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn parse_if_with_else_yields_else_body() {
        let stmt = parse_single_stmt("if 1 then print(1) else print(2) end").expect("must parse");
        match stmt.kind {
            StmtKind::If {
                then_body,
                elifs,
                else_body,
                ..
            } => {
                assert_eq!(then_body.len(), 1);
                assert!(elifs.is_empty());
                let body = else_body.expect("else body must exist");
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn parse_if_with_elseif_chain() {
        let stmt = parse_single_stmt(
            "if 1 then print(1) elseif 2 then print(2) elseif 3 then print(3) else print(0) end",
        )
        .expect("must parse");
        match stmt.kind {
            StmtKind::If {
                then_body,
                elifs,
                else_body,
                ..
            } => {
                assert_eq!(then_body.len(), 1);
                assert_eq!(elifs.len(), 2, "two elseif arms expected");
                assert!(else_body.is_some());
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn parse_while_yields_while_stmt() {
        let stmt = parse_single_stmt("while 1 do print(1) end").expect("must parse");
        match stmt.kind {
            StmtKind::While { body, .. } => {
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected While, got {other:?}"),
        }
    }

    #[test]
    fn parse_unterminated_if_is_rejected() {
        let err = parse("if 1 then print(1)").expect_err("missing `end` must fail");
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    #[test]
    fn parse_not_yields_unary_not() {
        let kind = parse_single_expr("not true").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(bool_lit(true)),
            },
        );
    }

    #[test]
    fn parse_and_binds_tighter_than_or() {
        // a or b and c == a or (b and c)
        let kind = parse_single_expr("true or false and true").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Or,
                lhs: Box::new(bool_lit(true)),
                rhs: Box::new(binop(BinOp::And, bool_lit(false), bool_lit(true))),
            },
        );
    }

    #[test]
    fn parse_or_lower_prec_than_and() {
        // a and b or c == (a and b) or c
        let kind = parse_single_expr("true and false or true").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Or,
                lhs: Box::new(binop(BinOp::And, bool_lit(true), bool_lit(false))),
                rhs: Box::new(bool_lit(true)),
            },
        );
    }

    #[test]
    fn parse_and_lower_prec_than_comparison() {
        // 1 < 2 and 3 < 4 == (1<2) and (3<4)
        let kind = parse_single_expr("1 < 2 and 3 < 4").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::And,
                lhs: Box::new(binop(BinOp::Lt, number(1.0), number(2.0))),
                rhs: Box::new(binop(BinOp::Lt, number(3.0), number(4.0))),
            },
        );
    }

    #[test]
    fn parse_not_with_truthy_subject() {
        // `not 1 < 2` == `(not 1) < 2`? Actually Lua: `not` is unary at
        // PREC_UNARY (12), `<` at PREC_CMP (8). So `not 1 < 2` parses
        // as `(not 1) < 2` (not binds tighter).
        let kind = parse_single_expr("not 1 < 2").expect("must parse");
        assert_eq!(
            kind,
            ExprKind::BinOp {
                op: BinOp::Lt,
                lhs: Box::new(unary(UnaryOp::Not, number(1.0))),
                rhs: Box::new(number(2.0)),
            },
        );
    }

    #[test]
    fn parse_unterminated_do_block_is_rejected() {
        let err = parse("do local x = 1").expect_err("missing `end` must fail");
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }
}
