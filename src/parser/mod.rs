//! Parser — turns a [`crate::lexer`] token stream into an AST.
//!
//! Public API is a single pure function [`parse`]. See
//! `docs/design/0004-parser-implementation.md` for the implementation
//! strategy (hand-written recursive descent with Pratt parsing) and
//! `docs/design/0007-phase2-0-local-and-multistmt.md` for the
//! statement-level extensions added in Phase 2.0.

mod ast;
mod error;

pub use ast::{BinOp, Chunk, Expr, ExprKind, Stmt, StmtKind, TableField, UnaryOp};
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
            TokenKind::Keyword(Keyword::Repeat) => self.parse_repeat(),
            TokenKind::Keyword(Keyword::For) => self.parse_for(),
            TokenKind::Keyword(Keyword::Break) => {
                let tok = self.bump().clone();
                Ok(Stmt::new(StmtKind::Break, tok.span))
            }
            TokenKind::Keyword(Keyword::Return) => self.parse_return(),
            // Phase 2.6+-methods (ADR 0092): top-level method-def
            // takes the `function NAME.field(...) end` or
            // `function NAME:method(...) end` shape — the
            // distinguishing lookahead is `Keyword::Function` followed
            // by `Ident`. A bare `function() ... end` at expression
            // position must continue to flow through `parse_expr` →
            // `parse_primary`'s anonymous FunctionExpr arm; the
            // method-def arm fires only when the receiver Ident is
            // present. Bare top-level `function NAME(...) end` (no
            // dot/colon) is still rejected inside `parse_method_def`
            // with `UnexpectedToken { LParen }`, since globals are
            // out of MVP scope.
            TokenKind::Keyword(Keyword::Function)
                if matches!(self.peek_kind_at(1), Some(TokenKind::Ident(_))) =>
            {
                self.parse_method_def()
            }
            TokenKind::Ident(_) if matches!(self.peek_kind_at(1), Some(TokenKind::Equals)) => {
                self.parse_assign()
            }
            // Phase 2.1a (ADR 0049): `NAME, NAME ... = ...` is a
            // multi-target reassignment. The lookahead at offset 1
            // is `,`; we then walk forward to confirm an `=` lives
            // before any non-comma/non-ident token, otherwise this
            // isn't a multi-assign and we let the expression parser
            // take over (which will error with a clear diagnostic).
            TokenKind::Ident(_)
                if matches!(self.peek_kind_at(1), Some(TokenKind::Comma))
                    && self.is_multi_assign_lookahead() =>
            {
                self.parse_multi_assign()
            }
            _ => {
                // Phase 2.6a-wr (ADR 0055): the fallthrough now also
                // handles `target[key] = value` table element write.
                // Parse the prefix expression first; if `=` follows
                // and the prefix is a valid lvalue (currently only
                // `ExprKind::Index`), build an `IndexAssign`. A bare
                // expression-statement still wraps in `ExprStmt`.
                let expr = self.parse_expr(0)?;
                if matches!(self.peek().kind, TokenKind::Equals) {
                    let eq_tok = self.bump().clone();
                    let value = self.parse_expr(0)?;
                    return match expr.kind {
                        ExprKind::Index { target, key } => {
                            let span = Span::new(target.span.start, value.span.end);
                            Ok(Stmt::new(
                                StmtKind::IndexAssign {
                                    target: *target,
                                    key: *key,
                                    value,
                                },
                                span,
                            ))
                        }
                        _ => Err(ParseError::UnexpectedToken {
                            actual: TokenKind::Equals,
                            offset: eq_tok.span.start,
                        }),
                    };
                }
                let span = expr.span;
                Ok(Stmt::new(StmtKind::ExprStmt(expr), span))
            }
        }
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let for_tok = self.bump().clone(); // 'for'
        // Loop variable name.
        let name_tok = self.peek().clone();
        let var = match name_tok.kind {
            TokenKind::Ident(n) => {
                self.bump();
                n
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
        // Phase 2.8e-iter-ipairs (ADR 0078): if the next token is
        // `,` the form is a generic / ipairs-sugar for-loop
        // (`for IDX, VAL in ipairs(t) do ...`); otherwise it's the
        // original numeric for. A bare `for NAME in ...` is also
        // rejected here — Plan C requires both `idx` and `val`
        // names because `ipairs` always yields two values.
        if matches!(self.peek().kind, TokenKind::Comma) {
            self.bump(); // consume ','
            let val_tok = self.peek().clone();
            let val_name = match val_tok.kind {
                TokenKind::Ident(n) => {
                    self.bump();
                    n
                }
                TokenKind::Eof => {
                    return Err(ParseError::UnexpectedEof {
                        offset: val_tok.span.start,
                    });
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        actual: other,
                        offset: val_tok.span.start,
                    });
                }
            };
            let in_offset = self.peek().span.start;
            self.expect_keyword(Keyword::In)?;
            let first_expr = self.parse_expr(0)?;
            // Phase 2.8e-iter-generic (ADR 0085): if a comma follows
            // the first expression, this is a 3-tuple `iter, state,
            // ctl` form (full Lua 5.4 generic-for). Otherwise try
            // the existing ipairs / pairs sugars.
            let iter_match = if matches!(self.peek().kind, TokenKind::Comma) {
                self.bump(); // consume ','
                let state = self.parse_expr(0)?;
                self.expect_token(TokenKind::Comma)?;
                let ctl = self.parse_expr(0)?;
                IterMatch::Generic {
                    iter: first_expr,
                    state,
                    ctl,
                }
            } else {
                match unwrap_named_unary_call(first_expr, "ipairs") {
                    Ok(table_expr) => IterMatch::Ipairs(table_expr),
                    Err(other) => match unwrap_named_unary_call(other, "pairs") {
                        Ok(table_expr) => IterMatch::Pairs(table_expr),
                        Err(_) => {
                            return Err(ParseError::UnsupportedIterator { offset: in_offset });
                        }
                    },
                }
            };
            self.expect_keyword(Keyword::Do)?;
            let body =
                self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
            let end_tok = self.expect_keyword(Keyword::End)?;
            let span = Span::new(for_tok.span.start, end_tok.span.end);
            let stmt_kind = match iter_match {
                IterMatch::Ipairs(table_expr) => StmtKind::ForIpairs {
                    idx_name: var,
                    val_name,
                    table: table_expr,
                    body,
                },
                IterMatch::Pairs(table_expr) => StmtKind::ForPairs {
                    key_name: var,
                    val_name,
                    table: table_expr,
                    body,
                },
                IterMatch::Generic { iter, state, ctl } => StmtKind::ForGeneric {
                    names: vec![var, val_name],
                    iter,
                    state,
                    ctl,
                    body,
                },
            };
            return Ok(Stmt::new(stmt_kind, span));
        }
        if matches!(self.peek().kind, TokenKind::Keyword(Keyword::In)) {
            // ADR 0285 — N4-F-2a: 1-name generic-for. Desugar to
            // the existing 2-name `ForGeneric` shape with synthetic
            // `_for1_discard` placeholder + `state, ctl = nil, nil`.
            // The iter must still return `(TaggedValue, TaggedValue)`
            // per the existing 2-name contract — gmatch et al. are
            // expected to produce the discarded second value as nil.
            let in_tok = self.bump(); // consume `in`
            let iter_expr = self.parse_expr(0)?;
            self.expect_keyword(Keyword::Do)?;
            let body =
                self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
            let end_tok = self.expect_keyword(Keyword::End)?;
            let span = Span::new(for_tok.span.start, end_tok.span.end);
            let nil_state = Expr::new(ExprKind::Nil, in_tok.span);
            let nil_ctl = Expr::new(ExprKind::Nil, in_tok.span);
            return Ok(Stmt::new(
                StmtKind::ForGeneric {
                    names: vec![var, "_for1_discard".to_owned()],
                    iter: iter_expr,
                    state: nil_state,
                    ctl: nil_ctl,
                    body,
                },
                span,
            ));
        }
        self.expect_token(TokenKind::Equals)?;
        let start = self.parse_expr(0)?;
        self.expect_token(TokenKind::Comma)?;
        let stop = self.parse_expr(0)?;
        let step = if matches!(self.peek().kind, TokenKind::Comma) {
            self.bump();
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect_keyword(Keyword::Do)?;
        let body = self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
        let end_tok = self.expect_keyword(Keyword::End)?;
        let span = Span::new(for_tok.span.start, end_tok.span.end);
        Ok(Stmt::new(
            StmtKind::ForNumeric {
                var,
                start,
                stop,
                step,
                body,
            },
            span,
        ))
    }

    fn expect_token(&mut self, expected: TokenKind) -> Result<Token, ParseError> {
        let tok = self.peek().clone();
        if tok.kind == expected {
            self.bump();
            Ok(tok)
        } else if matches!(tok.kind, TokenKind::Eof) {
            Err(ParseError::UnexpectedEof {
                offset: tok.span.start,
            })
        } else {
            Err(ParseError::UnexpectedToken {
                actual: tok.kind,
                offset: tok.span.start,
            })
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

    fn parse_repeat(&mut self) -> Result<Stmt, ParseError> {
        let repeat_tok = self.bump().clone();
        let body = self.parse_chunk_until(&[TokenKind::Keyword(Keyword::Until), TokenKind::Eof])?;
        self.expect_keyword(Keyword::Until)?;
        let cond = self.parse_expr(0)?;
        let span = Span::new(repeat_tok.span.start, cond.span.end);
        Ok(Stmt::new(StmtKind::Repeat { body, cond }, span))
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

    /// Phase 2.1a (ADR 0049): peek ahead to decide whether
    /// `Ident, Ident, …` is the start of a multi-target assignment
    /// (rather than e.g. an expression statement that starts with
    /// a comma-something later). The pattern we accept is
    /// `Ident (, Ident)+ =`. If the trailing `=` isn't there, the
    /// dispatcher falls back to expression parsing.
    fn is_multi_assign_lookahead(&self) -> bool {
        let mut idx = 0usize;
        loop {
            // Position 2*idx must be an Ident.
            match self.peek_kind_at(idx * 2) {
                Some(TokenKind::Ident(_)) => {}
                _ => return false,
            }
            // Position 2*idx + 1 is either `,` (continue) or `=` (done).
            match self.peek_kind_at(idx * 2 + 1) {
                Some(TokenKind::Comma) => {
                    idx += 1;
                    if idx > 32 {
                        // sanity bound
                        return false;
                    }
                }
                Some(TokenKind::Equals) => return idx >= 1,
                _ => return false,
            }
        }
    }

    fn parse_multi_assign(&mut self) -> Result<Stmt, ParseError> {
        let start_offset = self.peek().span.start;
        let mut names: Vec<String> = Vec::new();
        loop {
            let tok = self.peek().clone();
            let TokenKind::Ident(name) = tok.kind else {
                return Err(ParseError::UnexpectedToken {
                    actual: tok.kind,
                    offset: tok.span.start,
                });
            };
            self.bump(); // ident
            names.push(name);
            match self.peek().kind {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::Equals => {
                    self.bump();
                    break;
                }
                _ => {
                    let t = self.peek().clone();
                    return Err(ParseError::UnexpectedToken {
                        actual: t.kind,
                        offset: t.span.start,
                    });
                }
            }
        }
        let mut values: Vec<Expr> = Vec::new();
        values.push(self.parse_expr(0)?);
        while matches!(self.peek().kind, TokenKind::Comma) {
            self.bump();
            values.push(self.parse_expr(0)?);
        }
        let end_offset = values.last().map(|v| v.span.end).unwrap_or(start_offset);
        Ok(Stmt::new(
            StmtKind::AssignMulti { names, values },
            Span::new(start_offset, end_offset),
        ))
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
        // `local function NAME(PARAMS) BODY end` is the only local
        // function form in Phase 2.5a. Dispatch on the next token.
        if matches!(self.peek().kind, TokenKind::Keyword(Keyword::Function)) {
            return self.parse_function_def(local_tok);
        }
        // Parse one or more names separated by commas. Each name
        // may carry an attribute `<const>` or `<close>` per Lua
        // 5.4 §3.3.7 (ADR 0236).
        let mut names: Vec<String> = Vec::new();
        let mut attrs: Vec<Option<String>> = Vec::new();
        loop {
            let name_tok = self.peek().clone();
            match name_tok.kind {
                TokenKind::Ident(ref n) => {
                    self.bump();
                    names.push(n.clone());
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
            }
            // Optional attribute: `<IDENT>` immediately after the
            // name.
            let attr = if matches!(self.peek().kind, TokenKind::Lt) {
                self.bump();
                let inner_tok = self.peek().clone();
                let attr_name = match inner_tok.kind {
                    TokenKind::Ident(n) => {
                        self.bump();
                        n
                    }
                    other => {
                        return Err(ParseError::UnexpectedToken {
                            actual: other,
                            offset: inner_tok.span.start,
                        });
                    }
                };
                let gt_tok = self.peek().clone();
                match gt_tok.kind {
                    TokenKind::Gt => {
                        self.bump();
                    }
                    other => {
                        return Err(ParseError::UnexpectedToken {
                            actual: other,
                            offset: gt_tok.span.start,
                        });
                    }
                }
                Some(attr_name)
            } else {
                None
            };
            attrs.push(attr);
            if matches!(self.peek().kind, TokenKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
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
        // Parse one or more values separated by commas.
        let mut values: Vec<Expr> = vec![self.parse_expr(0)?];
        while matches!(self.peek().kind, TokenKind::Comma) {
            self.bump();
            values.push(self.parse_expr(0)?);
        }
        let span_end = values.last().expect("at least one value").span.end;
        let span = Span::new(local_tok.span.start, span_end);
        // Single name + single value → existing `Local` shape; any
        // multi shape (multi-name and/or multi-value) → `LocalMulti`,
        // which HIR validates against the Phase 2.5d rules.
        let kind = if names.len() == 1 && values.len() == 1 {
            let value = values.pop().unwrap();
            let name = names.pop().unwrap();
            let attr = attrs.pop().unwrap();
            StmtKind::Local { name, value, attr }
        } else {
            StmtKind::LocalMulti {
                names,
                values,
                attrs,
            }
        };
        Ok(Stmt::new(kind, span))
    }

    /// Phase 2.6+-methods (ADR 0092 / extended in ADR 0096): top-level
    /// `function recv[.field]*.method(...) end` or
    /// `function recv[.field]*:method(...) end`. Receiver chain is
    /// composed of dotted Ident segments; the final separator (`.`
    /// or `:`) determines `is_colon`. ADR 0095's TAG_TABLE narrow
    /// makes nested table targets safe at codegen.
    ///
    /// Rejects bare top-level `function NAME(...) end` (no receiver
    /// dot/colon) — that form requires globals, which are not yet
    /// supported. The rejection surfaces as
    /// `ParseError::UnexpectedToken { actual: LParen, ... }`.
    fn parse_method_def(&mut self) -> Result<Stmt, ParseError> {
        let fn_tok = self.bump().clone(); // 'function'
        // Consume the head Ident.
        let mut segments: Vec<String> = Vec::new();
        let head_tok = self.peek().clone();
        match head_tok.kind {
            TokenKind::Ident(ref n) => {
                segments.push(n.clone());
                self.bump();
            }
            TokenKind::Eof => {
                return Err(ParseError::UnexpectedEof {
                    offset: head_tok.span.start,
                });
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    actual: other,
                    offset: head_tok.span.start,
                });
            }
        }
        // Loop over `.IDENT` segments. The terminating separator is
        // either `:IDENT` (colon — sets is_colon=true and the next
        // Ident is the method) or LParen (the last consumed Ident
        // becomes the method; receiver_chain is everything before it).
        let mut is_colon = false;
        loop {
            let sep_tok = self.peek().clone();
            match sep_tok.kind {
                TokenKind::Dot => {
                    self.bump();
                    let id_tok = self.peek().clone();
                    match id_tok.kind {
                        TokenKind::Ident(ref n) => {
                            segments.push(n.clone());
                            self.bump();
                        }
                        TokenKind::Eof => {
                            return Err(ParseError::UnexpectedEof {
                                offset: id_tok.span.start,
                            });
                        }
                        other => {
                            return Err(ParseError::UnexpectedToken {
                                actual: other,
                                offset: id_tok.span.start,
                            });
                        }
                    }
                }
                TokenKind::Colon => {
                    self.bump();
                    let id_tok = self.peek().clone();
                    match id_tok.kind {
                        TokenKind::Ident(ref n) => {
                            segments.push(n.clone());
                            self.bump();
                        }
                        TokenKind::Eof => {
                            return Err(ParseError::UnexpectedEof {
                                offset: id_tok.span.start,
                            });
                        }
                        other => {
                            return Err(ParseError::UnexpectedToken {
                                actual: other,
                                offset: id_tok.span.start,
                            });
                        }
                    }
                    is_colon = true;
                    break;
                }
                TokenKind::LParen => break,
                TokenKind::Eof => {
                    return Err(ParseError::UnexpectedEof {
                        offset: sep_tok.span.start,
                    });
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        actual: other,
                        offset: sep_tok.span.start,
                    });
                }
            }
        }
        // After the loop:
        //   - dotted form: segments holds receiver+method; final Ident is method.
        //   - colon form: segments holds receiver+method; final Ident (post-colon)
        //                  is the method, segments before is the receiver chain.
        //   - if segments.len() == 1: the single Ident IS the method — that's
        //                  the bare `function NAME()` form which requires
        //                  globals; reject with UnexpectedToken at the LParen.
        if segments.len() < 2 {
            // peek is currently at LParen; report it as the rejection
            // position (consistent with ADR 0092's
            // bare_top_level_function_rejected pin).
            let lparen_tok = self.peek().clone();
            return Err(ParseError::UnexpectedToken {
                actual: lparen_tok.kind,
                offset: lparen_tok.span.start,
            });
        }
        let method = segments.pop().expect("segments has ≥2 entries");
        let receiver_chain = segments;
        let (params, body, end_tok) = self.parse_function_signature_and_body()?;
        let span = Span::new(fn_tok.span.start, end_tok.span.end);
        Ok(Stmt::new(
            StmtKind::MethodDef {
                receiver_chain,
                method,
                is_colon,
                params,
                body,
            },
            span,
        ))
    }

    fn parse_function_def(&mut self, local_tok: Token) -> Result<Stmt, ParseError> {
        self.bump(); // 'function'
        let name_tok = self.peek().clone();
        let name = match name_tok.kind {
            TokenKind::Ident(n) => {
                self.bump();
                n
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
        let (params, body, end_tok) = self.parse_function_signature_and_body()?;
        let span = Span::new(local_tok.span.start, end_tok.span.end);
        Ok(Stmt::new(
            StmtKind::FunctionDef { name, params, body },
            span,
        ))
    }

    /// Parse `(params) body end` — shared between `parse_function_def`
    /// (named) and `parse_function_expr` (anonymous, Phase 2.5b).
    fn parse_function_signature_and_body(
        &mut self,
    ) -> Result<(Vec<String>, Chunk, Token), ParseError> {
        self.expect_token(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                let p_tok = self.peek().clone();
                match p_tok.kind {
                    TokenKind::Ident(n) => {
                        self.bump();
                        params.push(n);
                    }
                    other => {
                        return Err(ParseError::UnexpectedToken {
                            actual: other,
                            offset: p_tok.span.start,
                        });
                    }
                }
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_token(TokenKind::RParen)?;
        let body = self.parse_chunk_until(&[TokenKind::Keyword(Keyword::End), TokenKind::Eof])?;
        let end_tok = self.expect_keyword(Keyword::End)?;
        Ok((params, body, end_tok))
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let return_tok = self.bump().clone(); // `return`
        // `return` with no value: next token is a chunk terminator
        // (`end`, `else`, `elseif`, EOF) or a `;`.
        let has_value = !matches!(
            self.peek().kind,
            TokenKind::Eof
                | TokenKind::Semicolon
                | TokenKind::Keyword(Keyword::End)
                | TokenKind::Keyword(Keyword::Else)
                | TokenKind::Keyword(Keyword::Elseif)
        );
        if !has_value {
            let span = Span::new(return_tok.span.start, return_tok.span.end);
            return Ok(Stmt::new(StmtKind::Return { value: None }, span));
        }
        // One or more comma-separated return expressions (Phase 2.5d).
        let mut values: Vec<Expr> = vec![self.parse_expr(0)?];
        while matches!(self.peek().kind, TokenKind::Comma) {
            self.bump();
            values.push(self.parse_expr(0)?);
        }
        let span_end = values.last().expect("at least one value").span.end;
        let span = Span::new(return_tok.span.start, span_end);
        let kind = if values.len() == 1 {
            StmtKind::Return {
                value: Some(values.pop().unwrap()),
            }
        } else {
            StmtKind::ReturnMulti { values }
        };
        Ok(Stmt::new(kind, span))
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
            // Phase 2.2c (ADR 0022): standalone `~` in prefix position
            // is bitwise NOT. (`~=` is its own token; in infix position
            // a bare `~` is bitwise XOR.)
            TokenKind::Tilde => Some(UnaryOp::BitNot),
            // Phase 2.7a (ADR 0024): `#s` is the length operator.
            TokenKind::Hash => Some(UnaryOp::Len),
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
            // ADR 0209 Phase B opt-in — integer-syntax literals
            // route to ExprKind::Integer(i64), preserving full i64
            // precision through the AST. HIR lowers to
            // HirExprKind::Integer; codegen demotes via sitofp.
            TokenKind::Integer(value) => {
                self.bump();
                Ok(Expr::new(ExprKind::Integer(value), tok.span))
            }
            TokenKind::Str(ref s) => {
                self.bump();
                Ok(Expr::new(ExprKind::Str(s.clone()), tok.span))
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
            TokenKind::Keyword(Keyword::Function) => {
                self.bump();
                let (params, body, end_tok) = self.parse_function_signature_and_body()?;
                let span = Span::new(tok.span.start, end_tok.span.end);
                Ok(Expr::new(ExprKind::FunctionExpr { params, body }, span))
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
            // Phase 2.6a-min (ADR 0053) / 2.6a-arr (ADR 0054) /
            // ADR 0199: table constructor with three field shapes.
            // Comma OR semicolon separator; trailing separator OK.
            TokenKind::LBrace => {
                self.bump();
                let mut fields: Vec<TableField> = Vec::new();
                loop {
                    if matches!(self.peek().kind, TokenKind::RBrace) {
                        let closing = self.bump().clone();
                        let span = Span::new(tok.span.start, closing.span.end);
                        return Ok(Expr::new(ExprKind::Table(fields), span));
                    }
                    if matches!(self.peek().kind, TokenKind::Eof) {
                        return Err(ParseError::UnexpectedEof {
                            offset: self.peek().span.start,
                        });
                    }
                    // ADR 0199 — field-shape detection:
                    //   `[` ... `]` `=` ... → Keyed{ key, value }
                    //   Ident `=` ...        → Keyed{ key: Str, value }
                    //   otherwise            → Positional
                    let field = if matches!(self.peek().kind, TokenKind::LBracket) {
                        self.bump(); // `[`
                        let key = self.parse_expr(0)?;
                        match self.peek().kind {
                            TokenKind::RBracket => {
                                self.bump();
                            }
                            _ => {
                                let t = self.peek().clone();
                                return Err(ParseError::UnexpectedToken {
                                    actual: t.kind,
                                    offset: t.span.start,
                                });
                            }
                        }
                        match self.peek().kind {
                            TokenKind::Equals => {
                                self.bump();
                            }
                            _ => {
                                let t = self.peek().clone();
                                return Err(ParseError::UnexpectedToken {
                                    actual: t.kind,
                                    offset: t.span.start,
                                });
                            }
                        }
                        let value = self.parse_expr(0)?;
                        TableField::Keyed { key, value }
                    } else if matches!(self.peek().kind, TokenKind::Ident(_))
                        && matches!(
                            self.tokens.get(self.pos + 1).map(|t| &t.kind),
                            Some(TokenKind::Equals)
                        )
                    {
                        // ADR 0199 — named-key sugar `Name = exp` →
                        // Keyed { Str(Name), exp }.
                        let ident_tok = self.bump().clone();
                        let name = if let TokenKind::Ident(n) = &ident_tok.kind {
                            n.clone()
                        } else {
                            unreachable!("matched Ident above");
                        };
                        self.bump(); // `=`
                        let value = self.parse_expr(0)?;
                        let key = Expr::new(ExprKind::Str(name), ident_tok.span);
                        TableField::Keyed { key, value }
                    } else {
                        TableField::Positional(self.parse_expr(0)?)
                    };
                    fields.push(field);
                    match self.peek().kind {
                        TokenKind::Comma | TokenKind::Semicolon => {
                            self.bump();
                        }
                        TokenKind::RBrace => {
                            let closing = self.bump().clone();
                            let span = Span::new(tok.span.start, closing.span.end);
                            return Ok(Expr::new(ExprKind::Table(fields), span));
                        }
                        TokenKind::Eof => {
                            return Err(ParseError::UnexpectedEof {
                                offset: self.peek().span.start,
                            });
                        }
                        _ => {
                            let t = self.peek().clone();
                            return Err(ParseError::UnexpectedToken {
                                actual: t.kind,
                                offset: t.span.start,
                            });
                        }
                    }
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
        // Phase 2.6a-arr (ADR 0054): a primary expression can chain
        // any of:  `(args)`  call  /  `[expr]`  index suffix. Lua's
        // prefix-expression rule allows `f()[i]`, `t[i][j]`, and
        // `({1,2})[1]` to all parse here.
        loop {
            match self.peek().kind {
                TokenKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        args.push(self.parse_expr(0)?);
                        while matches!(self.peek().kind, TokenKind::Comma) {
                            self.bump();
                            args.push(self.parse_expr(0)?);
                        }
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
                TokenKind::LBracket => {
                    self.bump();
                    let key = self.parse_expr(0)?;
                    let closing = self.peek().clone();
                    match closing.kind {
                        TokenKind::RBracket => {
                            self.bump();
                            let span = Span::new(callee.span.start, closing.span.end);
                            callee = Expr::new(
                                ExprKind::Index {
                                    target: Box::new(callee),
                                    key: Box::new(key),
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
                // Phase 2.6b-hash (ADR 0058): `.IDENT` field
                // access. Sugar to `Index { target, key: Str("k") }`
                // so AST/HIR/codegen reuse the index path with
                // `key_kind == String` driving the hash branch.
                TokenKind::Dot => {
                    self.bump();
                    let name_tok = self.peek().clone();
                    let name = match name_tok.kind {
                        TokenKind::Ident(ref n) => {
                            let s = n.clone();
                            self.bump();
                            s
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
                    let key_expr = Expr::new(ExprKind::Str(name), name_tok.span);
                    let span = Span::new(callee.span.start, name_tok.span.end);
                    callee = Expr::new(
                        ExprKind::Index {
                            target: Box::new(callee),
                            key: Box::new(key_expr),
                        },
                        span,
                    );
                }
                // Phase 2.6+-methods (ADR 0092): `:IDENT(args)` method
                // call. Lua spec requires the args list to follow
                // immediately, so we parse `(args)` here as part of
                // the same suffix step rather than relying on a
                // subsequent loop iteration. Emits `MethodCall`;
                // HIR desugars to an Index-callee Call.
                TokenKind::Colon => {
                    self.bump();
                    let name_tok = self.peek().clone();
                    let method = match name_tok.kind {
                        TokenKind::Ident(ref n) => {
                            let s = n.clone();
                            self.bump();
                            s
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
                    self.expect_token(TokenKind::LParen)?;
                    let mut args = Vec::new();
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        args.push(self.parse_expr(0)?);
                        while matches!(self.peek().kind, TokenKind::Comma) {
                            self.bump();
                            args.push(self.parse_expr(0)?);
                        }
                    }
                    let closing = self.expect_token(TokenKind::RParen)?;
                    let span = Span::new(callee.span.start, closing.span.end);
                    callee = Expr::new(
                        ExprKind::MethodCall {
                            receiver: Box::new(callee),
                            method,
                            args,
                        },
                        span,
                    );
                }
                _ => break,
            }
        }
        Ok(callee)
    }
}

/// Precedence ladder per ADRs 0009, 0010, 0013, 0022 (Lua 5.4 §3.4.8
/// subset). The bitwise tier (0022) sits between comparison (`<`, `==`,
/// ...) and additive (`+`, `-`):
///
///   `or` < `and` < `<`/`==`/... < `|` < `~` < `&` < `<<`/`>>` < `+`/`-`
///   < `*`/`/`/`//`/`%` < unary < `^`.
const PREC_OR: u8 = 5;
const PREC_AND: u8 = 6;
const PREC_CMP: u8 = 8;
const PREC_BOR: u8 = 9;
const PREC_BXOR: u8 = 10;
const PREC_BAND: u8 = 11;
const PREC_SHIFT: u8 = 12;
/// `..` string concat (Phase 2.7b, ADR 0025): right-associative,
/// between SHIFT and ADD per Lua 5.4 §3.4.8.
const PREC_CONCAT: u8 = 13;
const PREC_ADD: u8 = 14;
const PREC_MUL: u8 = 15;
const PREC_UNARY: u8 = 16;
const PREC_POW: u8 = 17;

struct InfixInfo {
    prec: u8,
    right_assoc: bool,
}

/// `ipairs` (ADR 0078) / `pairs` (ADR 0080) / generic-for (ADR 0085)
/// parser sugar dispatch. `parse_for` records which iterator shape
/// matched so the right statement kind is built without re-walking
/// the expression.
enum IterMatch {
    Ipairs(Expr),
    Pairs(Expr),
    Generic { iter: Expr, state: Expr, ctl: Expr },
}

/// Pattern-match `NAME(table_expr)` where `NAME` is `"ipairs"` or
/// `"pairs"`. Returns `Ok(table_expr)` on a single-argument match;
/// returns `Err(original_expr)` unchanged so the caller can re-try
/// with a different name without losing the parsed expression.
fn unwrap_named_unary_call(expr: Expr, name: &str) -> Result<Expr, Expr> {
    let matches = matches!(
        &expr.kind,
        ExprKind::Call { callee, args }
            if matches!(&callee.kind, ExprKind::Ident(n) if n == name)
                && args.len() == 1
    );
    if !matches {
        return Err(expr);
    }
    match expr.kind {
        ExprKind::Call { args, .. } => {
            let mut args = args;
            Ok(args.remove(0))
        }
        _ => unreachable!(),
    }
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
        TokenKind::Pipe => Some(InfixInfo {
            prec: PREC_BOR,
            right_assoc: false,
        }),
        TokenKind::Tilde => Some(InfixInfo {
            prec: PREC_BXOR,
            right_assoc: false,
        }),
        TokenKind::Amp => Some(InfixInfo {
            prec: PREC_BAND,
            right_assoc: false,
        }),
        TokenKind::LtLt | TokenKind::GtGt => Some(InfixInfo {
            prec: PREC_SHIFT,
            right_assoc: false,
        }),
        TokenKind::DotDot => Some(InfixInfo {
            prec: PREC_CONCAT,
            right_assoc: true,
        }),
        TokenKind::Plus | TokenKind::Minus => Some(InfixInfo {
            prec: PREC_ADD,
            right_assoc: false,
        }),
        TokenKind::Star | TokenKind::Slash | TokenKind::SlashSlash | TokenKind::Percent => {
            Some(InfixInfo {
                prec: PREC_MUL,
                right_assoc: false,
            })
        }
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
        TokenKind::SlashSlash => Some(BinOp::FloorDiv),
        TokenKind::Percent => Some(BinOp::Mod),
        TokenKind::Caret => Some(BinOp::Pow),
        TokenKind::Amp => Some(BinOp::BitAnd),
        TokenKind::Pipe => Some(BinOp::BitOr),
        TokenKind::Tilde => Some(BinOp::BitXor),
        TokenKind::LtLt => Some(BinOp::Shl),
        TokenKind::GtGt => Some(BinOp::Shr),
        TokenKind::DotDot => Some(BinOp::Concat),
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

    /// ADR 0209 — currently unused. Was the default literal helper
    /// for parser tests; after Phase B all whole-number test inputs
    /// migrated to `integer()`. Retained for fractional-literal tests
    /// when those are added.
    #[allow(dead_code)]
    fn number(v: f64) -> Expr {
        Expr::new(ExprKind::Number(v), Span::new(0, 0))
    }

    /// ADR 0209 — test helper for integer-syntax literals. Most
    /// existing parser tests pass whole-number sources (`42`,
    /// `1`, `2`); under Phase B these now parse as
    /// `ExprKind::Integer(N)` not `ExprKind::Number(N.0)`. Tests
    /// migrated to `integer(N)` for source-fidelity.
    fn integer(v: i64) -> Expr {
        Expr::new(ExprKind::Integer(v), Span::new(0, 0))
    }

    fn ident(name: &str) -> Expr {
        Expr::new(ExprKind::Ident(name.to_owned()), Span::new(0, 0))
    }

    fn strip_span_expr(expr: Expr) -> Expr {
        let kind = match expr.kind {
            ExprKind::Number(v) => ExprKind::Number(v),
            // ADR 0209 — Integer preserved through span strip.
            ExprKind::Integer(v) => ExprKind::Integer(v),
            ExprKind::Str(s) => ExprKind::Str(s),
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
            ExprKind::FunctionExpr { params, body } => ExprKind::FunctionExpr {
                params,
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            ExprKind::Table(fields) => ExprKind::Table(
                fields
                    .into_iter()
                    .map(|f| match f {
                        TableField::Positional(e) => TableField::Positional(strip_span_expr(e)),
                        TableField::Keyed { key, value } => TableField::Keyed {
                            key: strip_span_expr(key),
                            value: strip_span_expr(value),
                        },
                    })
                    .collect(),
            ),
            ExprKind::Index { target, key } => ExprKind::Index {
                target: Box::new(strip_span_expr(*target)),
                key: Box::new(strip_span_expr(*key)),
            },
            ExprKind::MethodCall {
                receiver,
                method,
                args,
            } => ExprKind::MethodCall {
                receiver: Box::new(strip_span_expr(*receiver)),
                method,
                args: args.into_iter().map(strip_span_expr).collect(),
            },
        };
        Expr::new(kind, Span::new(0, 0))
    }

    fn strip_span_stmt(stmt: Stmt) -> Stmt {
        let kind = match stmt.kind {
            StmtKind::Local { name, value, attr } => StmtKind::Local {
                name,
                value: strip_span_expr(value),
                attr,
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
            StmtKind::Repeat { body, cond } => StmtKind::Repeat {
                body: body.into_iter().map(strip_span_stmt).collect(),
                cond: strip_span_expr(cond),
            },
            StmtKind::ForNumeric {
                var,
                start,
                stop,
                step,
                body,
            } => StmtKind::ForNumeric {
                var,
                start: strip_span_expr(start),
                stop: strip_span_expr(stop),
                step: step.map(strip_span_expr),
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::ForIpairs {
                idx_name,
                val_name,
                table,
                body,
            } => StmtKind::ForIpairs {
                idx_name,
                val_name,
                table: strip_span_expr(table),
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::ForPairs {
                key_name,
                val_name,
                table,
                body,
            } => StmtKind::ForPairs {
                key_name,
                val_name,
                table: strip_span_expr(table),
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::ForGeneric {
                names,
                iter,
                state,
                ctl,
                body,
            } => StmtKind::ForGeneric {
                names,
                iter: strip_span_expr(iter),
                state: strip_span_expr(state),
                ctl: strip_span_expr(ctl),
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::Break => StmtKind::Break,
            StmtKind::FunctionDef { name, params, body } => StmtKind::FunctionDef {
                name,
                params,
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::MethodDef {
                receiver_chain,
                method,
                is_colon,
                params,
                body,
            } => StmtKind::MethodDef {
                receiver_chain,
                method,
                is_colon,
                params,
                body: body.into_iter().map(strip_span_stmt).collect(),
            },
            StmtKind::Return { value } => StmtKind::Return {
                value: value.map(strip_span_expr),
            },
            StmtKind::LocalMulti {
                names,
                values,
                attrs,
            } => StmtKind::LocalMulti {
                names,
                values: values.into_iter().map(strip_span_expr).collect(),
                attrs,
            },
            StmtKind::AssignMulti { names, values } => StmtKind::AssignMulti {
                names,
                values: values.into_iter().map(strip_span_expr).collect(),
            },
            StmtKind::IndexAssign { target, key, value } => StmtKind::IndexAssign {
                target: strip_span_expr(target),
                key: strip_span_expr(key),
                value: strip_span_expr(value),
            },
            StmtKind::ReturnMulti { values } => StmtKind::ReturnMulti {
                values: values.into_iter().map(strip_span_expr).collect(),
            },
            StmtKind::ExprStmt(e) => StmtKind::ExprStmt(strip_span_expr(e)),
        };
        Stmt::new(kind, Span::new(0, 0))
    }

    #[test]
    fn parse_integer_literal_yields_integer_expr() {
        // ADR 0209 — Phase B: integer-syntax literals route to
        // ExprKind::Integer(i64). HIR demotes to Number for the
        // 125 ValueKind::Number consumer surface (Phase B silent
        // demotion); codegen emits sitofp.
        assert_eq!(parse_single_expr("42").unwrap(), ExprKind::Integer(42));
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
                lhs: Box::new(integer(1)),
                rhs: Box::new(integer(2)),
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
                        lhs: Box::new(integer(1)),
                        rhs: Box::new(integer(2)),
                    },
                    Span::new(0, 0),
                )),
                rhs: Box::new(integer(3)),
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
                args: vec![integer(42)],
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
                        lhs: Box::new(integer(1)),
                        rhs: Box::new(integer(2)),
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
                value: integer(1),
                attr: None,
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
                value: integer(1),
                attr: None,
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
                value: integer(2),
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
                lhs: Box::new(integer(3)),
                rhs: Box::new(integer(1)),
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
                lhs: Box::new(integer(1)),
                rhs: Box::new(binop(BinOp::Mul, integer(2), integer(3))),
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
                lhs: Box::new(binop(BinOp::Div, integer(6), integer(2))),
                rhs: Box::new(integer(3)),
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
                lhs: Box::new(integer(2)),
                rhs: Box::new(binop(BinOp::Pow, integer(3), integer(2))),
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
                operand: Box::new(integer(5)),
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
                operand: Box::new(binop(BinOp::Pow, integer(2), integer(3))),
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
                lhs: Box::new(unary(UnaryOp::Neg, integer(2))),
                rhs: Box::new(integer(3)),
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
                lhs: Box::new(integer(1)),
                rhs: Box::new(integer(2)),
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
                lhs: Box::new(integer(1)),
                rhs: Box::new(integer(1)),
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
                lhs: Box::new(binop(BinOp::Add, integer(1), integer(2))),
                rhs: Box::new(binop(BinOp::Mul, integer(3), integer(4))),
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
                lhs: Box::new(binop(BinOp::Lt, integer(1), integer(2))),
                rhs: Box::new(binop(BinOp::Lt, integer(3), integer(4))),
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
                lhs: Box::new(unary(UnaryOp::Not, integer(1))),
                rhs: Box::new(integer(2)),
            },
        );
    }

    #[test]
    fn parse_for_with_implicit_step() {
        let stmt = parse_single_stmt("for i = 1, 3 do print(i) end").expect("must parse");
        match stmt.kind {
            StmtKind::ForNumeric {
                var, step, body, ..
            } => {
                assert_eq!(var, "i");
                assert!(step.is_none());
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected ForNumeric, got {other:?}"),
        }
    }

    #[test]
    fn parse_for_with_explicit_step() {
        let stmt = parse_single_stmt("for i = 10, 1, -2 do print(i) end").expect("must parse");
        match stmt.kind {
            StmtKind::ForNumeric { step, .. } => {
                assert!(step.is_some());
            }
            other => panic!("expected ForNumeric, got {other:?}"),
        }
    }

    #[test]
    fn parse_for_missing_do_is_rejected() {
        let err = parse("for i = 1, 3 print(i) end").expect_err("missing `do` must fail");
        assert!(matches!(err, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn parse_for_missing_comma_is_rejected() {
        let err = parse("for i = 1 do print(i) end").expect_err("missing comma must fail");
        assert!(matches!(err, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn parse_for_unterminated_is_rejected() {
        let err = parse("for i = 1, 3 do print(i)").expect_err("missing `end` must fail");
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    #[test]
    fn parse_break_yields_break_stmt() {
        let stmt = parse_single_stmt("break").expect("must parse");
        assert!(matches!(stmt.kind, StmtKind::Break));
    }

    #[test]
    fn parse_break_inside_while_body() {
        let stmt = parse_single_stmt("while true do break end").expect("must parse");
        match stmt.kind {
            StmtKind::While { body, .. } => {
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0].kind, StmtKind::Break));
            }
            other => panic!("expected While, got {other:?}"),
        }
    }

    #[test]
    fn parse_local_function_no_args_no_return_yields_function_def() {
        let stmt = parse_single_stmt("local function f() end").expect("must parse");
        match stmt.kind {
            StmtKind::FunctionDef {
                name, params, body, ..
            } => {
                assert_eq!(name, "f");
                assert!(params.is_empty());
                assert!(body.is_empty());
            }
            other => panic!("expected FunctionDef, got {other:?}"),
        }
    }

    #[test]
    fn parse_local_function_with_params_and_return() {
        let stmt =
            parse_single_stmt("local function add(a, b) return a + b end").expect("must parse");
        match stmt.kind {
            StmtKind::FunctionDef { name, params, body } => {
                assert_eq!(name, "add");
                assert_eq!(params, vec!["a".to_owned(), "b".to_owned()]);
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0].kind, StmtKind::Return { value: Some(_) }));
            }
            other => panic!("expected FunctionDef, got {other:?}"),
        }
    }

    #[test]
    fn parse_return_with_value() {
        // `return` is only valid inside a function body in HIR, but
        // the parser accepts it everywhere — semantics gate is HIR.
        let chunk = parse("local function f() return 42 end").expect("must parse");
        let StmtKind::FunctionDef { body, .. } = &chunk[0].kind else {
            panic!("expected FunctionDef");
        };
        match &body[0].kind {
            StmtKind::Return { value: Some(_) } => {}
            other => panic!("expected Return with value, got {other:?}"),
        }
    }

    #[test]
    fn parse_return_without_value() {
        let chunk = parse("local function f() return end").expect("must parse");
        let StmtKind::FunctionDef { body, .. } = &chunk[0].kind else {
            panic!("expected FunctionDef");
        };
        match &body[0].kind {
            StmtKind::Return { value: None } => {}
            other => panic!("expected Return without value, got {other:?}"),
        }
    }

    #[test]
    fn parse_local_function_missing_end_is_rejected() {
        let err = parse("local function f()").expect_err("missing `end` must fail");
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    #[test]
    fn parse_anonymous_function_expr_yields_function_expr() {
        let kind = parse_single_expr("function() return 1 end").expect("must parse");
        match kind {
            ExprKind::FunctionExpr { params, body } => {
                assert!(params.is_empty());
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected FunctionExpr, got {other:?}"),
        }
    }

    #[test]
    fn parse_function_expr_in_local_init() {
        let stmt = parse_single_stmt("local f = function(x) return x end").expect("must parse");
        match stmt.kind {
            StmtKind::Local { name, value, .. } => {
                assert_eq!(name, "f");
                assert!(matches!(value.kind, ExprKind::FunctionExpr { .. }));
            }
            other => panic!("expected Local with FunctionExpr, got {other:?}"),
        }
    }

    #[test]
    fn parse_function_expr_with_params_and_body() {
        let kind = parse_single_expr("function(a, b) return a + b end").expect("must parse");
        match kind {
            ExprKind::FunctionExpr { params, body } => {
                assert_eq!(params, vec!["a".to_owned(), "b".to_owned()]);
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected FunctionExpr, got {other:?}"),
        }
    }

    #[test]
    fn parse_unterminated_do_block_is_rejected() {
        let err = parse("do local x = 1").expect_err("missing `end` must fail");
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }

    // -----------------------------------------------------------
    // Phase 2.5d — multi-return / multi-binding (ADR 0021).
    // -----------------------------------------------------------

    #[test]
    fn parse_local_multi_value_yields_local_multi() {
        let stmt = parse_single_stmt("local a, b = 1, 2").expect("must parse");
        match stmt.kind {
            StmtKind::LocalMulti { names, values, .. } => {
                assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
                assert_eq!(values.len(), 2);
            }
            other => panic!("expected LocalMulti, got {other:?}"),
        }
    }

    #[test]
    fn parse_local_multi_name_single_call_yields_local_multi() {
        let stmt = parse_single_stmt("local a, b = f()").expect("must parse");
        match stmt.kind {
            StmtKind::LocalMulti { names, values, .. } => {
                assert_eq!(names.len(), 2);
                assert_eq!(values.len(), 1);
                assert!(matches!(values[0].kind, ExprKind::Call { .. }));
            }
            other => panic!("expected LocalMulti, got {other:?}"),
        }
    }

    #[test]
    fn parse_return_multi_values() {
        // `return 1, 2` parses as ReturnMulti { values: [1, 2] }.
        let stmt = parse("local function f() return 1, 2 end")
            .expect("must parse")
            .into_iter()
            .next()
            .unwrap();
        let StmtKind::FunctionDef { body, .. } = stmt.kind else {
            panic!("expected FunctionDef");
        };
        match &body[0].kind {
            StmtKind::ReturnMulti { values } => {
                assert_eq!(values.len(), 2);
            }
            other => panic!("expected ReturnMulti, got {other:?}"),
        }
    }

    #[test]
    fn parse_single_return_unchanged_after_2_5d() {
        // Regression: single-return uses the existing Return shape.
        let stmt = parse_single_stmt("local function f() return 42 end").expect("must parse");
        let StmtKind::FunctionDef { body, .. } = stmt.kind else {
            panic!("expected FunctionDef");
        };
        match &body[0].kind {
            StmtKind::Return { value: Some(_) } => {}
            other => panic!("expected Return (single), got {other:?}"),
        }
    }
}
