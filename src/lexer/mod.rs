//! Lexer — turns a Lua source string into a flat stream of [`Token`]s.
//!
//! Public API is a single pure function [`lex`]. See
//! `docs/design/0001-lexer-implementation.md` for the implementation
//! strategy (hand-written, no combinator or derive-macro crate).

mod error;
mod token;

pub use error::LexError;
pub use token::{Keyword, Span, Token, TokenKind};

/// Tokenize a Lua source string.
///
/// Pure function: identical input always yields identical output.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let mut tokens = Vec::new();
    let mut chars = src.char_indices().peekable();

    while let Some(&(offset, ch)) = chars.peek() {
        if ch.is_ascii_whitespace() {
            chars.next();
            continue;
        }

        if ch.is_ascii_digit() {
            let (span, value) = scan_number(src, &mut chars);
            tokens.push(Token::new(TokenKind::Number(value), span));
            continue;
        }

        if is_ident_start(ch) {
            let (span, name) = scan_ident(src, &mut chars);
            let kind = match Keyword::from_lexeme(&name) {
                Some(kw) => TokenKind::Keyword(kw),
                None => TokenKind::Ident(name),
            };
            tokens.push(Token::new(kind, span));
            continue;
        }

        // Two-character punctuation with `=` as the second char.
        // `=`/`<`/`>`/`~` may form `==`/`<=`/`>=`/`~=`. `~` alone is
        // reserved for Phase 2.4 bitwise NOT and rejected here.
        if matches!(ch, '=' | '<' | '>' | '~') {
            chars.next();
            let next_eq = matches!(chars.peek(), Some(&(_, '=')));
            let kind = match (ch, next_eq) {
                ('=', true) => TokenKind::EqEq,
                ('=', false) => TokenKind::Equals,
                ('<', true) => TokenKind::LtEq,
                ('<', false) => TokenKind::Lt,
                ('>', true) => TokenKind::GtEq,
                ('>', false) => TokenKind::Gt,
                ('~', true) => TokenKind::TildeEq,
                ('~', false) => return Err(LexError::Unexpected { ch, offset }),
                _ => unreachable!(),
            };
            let end = if next_eq {
                chars.next();
                offset + ch.len_utf8() + '='.len_utf8()
            } else {
                offset + ch.len_utf8()
            };
            tokens.push(Token::new(kind, Span::new(offset, end)));
            continue;
        }

        let single = match ch {
            '(' => Some(TokenKind::LParen),
            ')' => Some(TokenKind::RParen),
            '+' => Some(TokenKind::Plus),
            '-' => Some(TokenKind::Minus),
            '*' => Some(TokenKind::Star),
            '/' => Some(TokenKind::Slash),
            '%' => Some(TokenKind::Percent),
            '^' => Some(TokenKind::Caret),
            ';' => Some(TokenKind::Semicolon),
            _ => None,
        };

        if let Some(kind) = single {
            let span = Span::new(offset, offset + ch.len_utf8());
            tokens.push(Token::new(kind, span));
            chars.next();
            continue;
        }

        return Err(LexError::Unexpected { ch, offset });
    }

    let end = src.len();
    tokens.push(Token::new(TokenKind::Eof, Span::new(end, end)));
    Ok(tokens)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn scan_number<I>(src: &str, chars: &mut std::iter::Peekable<I>) -> (Span, f64)
where
    I: Iterator<Item = (usize, char)>,
{
    let &(start, _) = chars.peek().expect("scan_number requires a digit");
    let mut end = start;
    while let Some(&(offset, ch)) = chars.peek() {
        if ch.is_ascii_digit() {
            end = offset + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let lexeme = &src[start..end];
    let value = lexeme
        .parse::<f64>()
        .expect("ascii-digit run is always a valid f64");
    (Span::new(start, end), value)
}

fn scan_ident<I>(src: &str, chars: &mut std::iter::Peekable<I>) -> (Span, String)
where
    I: Iterator<Item = (usize, char)>,
{
    let &(start, _) = chars.peek().expect("scan_ident requires an ident-start");
    let mut end = start;
    while let Some(&(offset, ch)) = chars.peek() {
        if is_ident_continue(ch) {
            end = offset + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    (Span::new(start, end), src[start..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src)
            .expect("source must lex cleanly")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn lex_empty_source_yields_single_eof_token() {
        assert_eq!(kinds(""), vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_single_integer_literal_yields_number_token() {
        assert_eq!(kinds("42"), vec![TokenKind::Number(42.0), TokenKind::Eof]);
    }

    #[test]
    fn lex_plus_between_integers_yields_three_tokens() {
        assert_eq!(
            kinds("1+2"),
            vec![
                TokenKind::Number(1.0),
                TokenKind::Plus,
                TokenKind::Number(2.0),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_whitespace_is_skipped() {
        assert_eq!(
            kinds(" 1 +\t2\n"),
            vec![
                TokenKind::Number(1.0),
                TokenKind::Plus,
                TokenKind::Number(2.0),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_identifier_yields_ident_token() {
        assert_eq!(
            kinds("print"),
            vec![TokenKind::Ident("print".to_owned()), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_print_call_with_addition_yields_expected_token_sequence() {
        assert_eq!(
            kinds("print(1 + 2)"),
            vec![
                TokenKind::Ident("print".to_owned()),
                TokenKind::LParen,
                TokenKind::Number(1.0),
                TokenKind::Plus,
                TokenKind::Number(2.0),
                TokenKind::RParen,
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_local_keyword_yields_keyword_token() {
        assert_eq!(
            kinds("local"),
            vec![TokenKind::Keyword(Keyword::Local), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_equals_and_semicolon_yield_punctuation_tokens() {
        assert_eq!(
            kinds("local x = 1;"),
            vec![
                TokenKind::Keyword(Keyword::Local),
                TokenKind::Ident("x".to_owned()),
                TokenKind::Equals,
                TokenKind::Number(1.0),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_identifier_starting_with_keyword_prefix_is_still_ident() {
        assert_eq!(
            kinds("locale"),
            vec![TokenKind::Ident("locale".to_owned()), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_do_and_end_keywords_yield_keyword_tokens() {
        assert_eq!(
            kinds("do end"),
            vec![
                TokenKind::Keyword(Keyword::Do),
                TokenKind::Keyword(Keyword::End),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_nil_keyword_yields_keyword_token() {
        assert_eq!(
            kinds("nil"),
            vec![TokenKind::Keyword(Keyword::Nil), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_double_equals_yields_eqeq_token() {
        assert_eq!(
            kinds("=="),
            vec![TokenKind::EqEq, TokenKind::Eof],
            "`==` must lex as a single EqEq token, not two `=`",
        );
    }

    #[test]
    fn lex_lt_eq_yields_lteq_and_gt_eq_yields_gteq() {
        assert_eq!(
            kinds("<= >="),
            vec![TokenKind::LtEq, TokenKind::GtEq, TokenKind::Eof],
        );
    }

    #[test]
    fn lex_tilde_eq_yields_tildeeq() {
        assert_eq!(kinds("~="), vec![TokenKind::TildeEq, TokenKind::Eof],);
    }

    #[test]
    fn lex_lt_and_gt_alone_yield_single_tokens() {
        assert_eq!(
            kinds("< >"),
            vec![TokenKind::Lt, TokenKind::Gt, TokenKind::Eof],
        );
    }

    #[test]
    fn lex_tilde_alone_returns_unexpected_error() {
        let err = lex("~").expect_err("standalone `~` is reserved (bitwise NOT — Phase 2.4)");
        assert_eq!(err, LexError::Unexpected { ch: '~', offset: 0 });
    }

    #[test]
    fn lex_arithmetic_punctuation_yields_individual_tokens() {
        assert_eq!(
            kinds("- * / % ^"),
            vec![
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::Caret,
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_identifier_ending_with_keyword_suffix_is_still_ident() {
        assert_eq!(
            kinds("ended"),
            vec![TokenKind::Ident("ended".to_owned()), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_unknown_character_returns_unexpected_error() {
        let err = lex("@").expect_err("`@` is not a valid Lua token in Phase 1");
        assert_eq!(err, LexError::Unexpected { ch: '@', offset: 0 },);
    }

    #[test]
    fn lex_assigns_byte_spans_pointing_into_the_source() {
        let src = "print(1)";
        let tokens = lex(src).expect("source must lex cleanly");
        let lexemes: Vec<&str> = tokens
            .iter()
            .filter(|t| t.kind != TokenKind::Eof)
            .map(|t| &src[t.span.start..t.span.end])
            .collect();
        assert_eq!(lexemes, vec!["print", "(", "1", ")"]);
    }
}
