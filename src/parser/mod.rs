//! Parser — turns a [`crate::lexer`] token stream into an AST.
//!
//! Public API is a single pure function [`parse`]. See
//! `docs/design/0004-parser-implementation.md` for the implementation
//! strategy (hand-written recursive descent with Pratt parsing).

mod ast;
mod error;

pub use ast::{BinOp, Expr, ExprKind};
pub use error::ParseError;

use crate::lexer::{self, Span, Token, TokenKind};

/// Parse a Lua source string into an [`Expr`].
///
/// Pure function: identical input always yields identical output. Internally
/// runs the lexer first and propagates any [`crate::lexer::LexError`] via
/// [`ParseError::Lex`].
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let tokens = lexer::lex(src)?;
    let mut parser = Parser::new(&tokens);
    let expr = parser.parse_expr(0)?;
    parser.expect_eof()?;
    Ok(expr)
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

    fn parse_kind(src: &str) -> Result<ExprKind, ParseError> {
        parse(src).map(|e| e.kind)
    }

    fn number(v: f64) -> Expr {
        Expr::new(ExprKind::Number(v), Span::new(0, 0))
    }

    fn ident(name: &str) -> Expr {
        Expr::new(ExprKind::Ident(name.to_owned()), Span::new(0, 0))
    }

    fn strip_span(expr: Expr) -> Expr {
        let kind = match expr.kind {
            ExprKind::Number(v) => ExprKind::Number(v),
            ExprKind::Ident(n) => ExprKind::Ident(n),
            ExprKind::Call { callee, args } => ExprKind::Call {
                callee: Box::new(strip_span(*callee)),
                args: args.into_iter().map(strip_span).collect(),
            },
            ExprKind::BinOp { op, lhs, rhs } => ExprKind::BinOp {
                op,
                lhs: Box::new(strip_span(*lhs)),
                rhs: Box::new(strip_span(*rhs)),
            },
        };
        Expr::new(kind, Span::new(0, 0))
    }

    #[test]
    fn parse_integer_literal_yields_number_expr() {
        assert_eq!(parse_kind("42").unwrap(), ExprKind::Number(42.0));
    }

    #[test]
    fn parse_identifier_yields_ident_expr() {
        assert_eq!(
            parse_kind("print").unwrap(),
            ExprKind::Ident("print".to_owned()),
        );
    }

    #[test]
    fn parse_simple_addition_yields_binop_expr() {
        let expr = parse("1 + 2").expect("addition must parse");
        let expected = Expr::new(
            ExprKind::BinOp {
                op: BinOp::Add,
                lhs: Box::new(number(1.0)),
                rhs: Box::new(number(2.0)),
            },
            Span::new(0, 0),
        );
        assert_eq!(strip_span(expr), expected);
    }

    #[test]
    fn parse_left_associative_chain() {
        let expr = parse("1 + 2 + 3").expect("chained addition must parse");
        let expected = Expr::new(
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
            Span::new(0, 0),
        );
        assert_eq!(strip_span(expr), expected);
    }

    #[test]
    fn parse_call_without_args_yields_call_expr() {
        let expr = parse("f()").expect("zero-arg call must parse");
        let expected = Expr::new(
            ExprKind::Call {
                callee: Box::new(ident("f")),
                args: vec![],
            },
            Span::new(0, 0),
        );
        assert_eq!(strip_span(expr), expected);
    }

    #[test]
    fn parse_call_with_single_arg() {
        let expr = parse("print(42)").expect("single-arg call must parse");
        let expected = Expr::new(
            ExprKind::Call {
                callee: Box::new(ident("print")),
                args: vec![number(42.0)],
            },
            Span::new(0, 0),
        );
        assert_eq!(strip_span(expr), expected);
    }

    #[test]
    fn parse_print_call_with_addition_matches_phase1_goal() {
        let expr = parse("print(1 + 2)").expect("Phase 1 target must parse");
        let expected = Expr::new(
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
            Span::new(0, 0),
        );
        assert_eq!(strip_span(expr), expected);
    }

    #[test]
    fn parse_call_span_covers_entire_call_site() {
        let src = "print(1 + 2)";
        let expr = parse(src).expect("Phase 1 target must parse");
        assert_eq!(expr.span, Span::new(0, src.len()));
        assert!(matches!(expr.kind, ExprKind::Call { .. }));
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
    fn parse_trailing_token_is_rejected() {
        let err = parse("1 2").expect_err("two expressions in a row must fail");
        assert_eq!(
            err,
            ParseError::UnexpectedToken {
                actual: TokenKind::Number(2.0),
                offset: 2,
            },
        );
    }
}
