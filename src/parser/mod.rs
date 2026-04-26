//! Parser — turns a [`crate::lexer`] token stream into an AST.
//!
//! Public API is a single pure function [`parse`]. See
//! `docs/design/0004-parser-implementation.md` for the implementation
//! strategy (hand-written recursive descent with Pratt parsing) and
//! `docs/design/0007-phase2-0-local-and-multistmt.md` for the
//! statement-level extensions added in Phase 2.0.

mod ast;
mod error;

pub use ast::{BinOp, Chunk, Expr, ExprKind, Stmt, StmtKind};
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
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::Semicolon) {
                self.bump();
            }
            if matches!(self.peek().kind, TokenKind::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        if matches!(self.peek().kind, TokenKind::Keyword(Keyword::Local)) {
            self.parse_local()
        } else {
            let expr = self.parse_expr(0)?;
            let span = expr.span;
            Ok(Stmt::new(StmtKind::ExprStmt(expr), span))
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
        let mut lhs = self.parse_primary()?;
        lhs = self.parse_call_suffix(lhs)?;

        while let Some(prec) = infix_precedence(&self.peek().kind) {
            if prec < min_prec {
                break;
            }
            let op_tok = self.bump();
            let op = binop_from_token(&op_tok.kind)
                .expect("infix_precedence guarantees a known operator");
            let rhs = self.parse_expr(prec + 1)?;
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

fn infix_precedence(kind: &TokenKind) -> Option<u8> {
    match kind {
        TokenKind::Plus => Some(10),
        _ => None,
    }
}

fn binop_from_token(kind: &TokenKind) -> Option<BinOp> {
    match kind {
        TokenKind::Plus => Some(BinOp::Add),
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
        };
        Expr::new(kind, Span::new(0, 0))
    }

    fn strip_span_stmt(stmt: Stmt) -> Stmt {
        let kind = match stmt.kind {
            StmtKind::Local { name, value } => StmtKind::Local {
                name,
                value: strip_span_expr(value),
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
}
