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

        if matches!(ch, '"' | '\'') {
            let (span, s) = scan_string(src, &mut chars, ch, offset)?;
            tokens.push(Token::new(TokenKind::Str(s), span));
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

        // Two-character punctuation. `=`/`~` may form `==`/`~=`;
        // `<`/`>` may form `<=`/`>=` or `<<`/`>>`; `/` may form `//`.
        // `.` (Phase 2.7b, ADR 0025) only forms `..`; the lone `.`
        // remains a lex error until field access lands.
        // `~` standalone is bitwise XOR/NOT (Phase 2.2c, ADR 0022).
        if matches!(ch, '=' | '<' | '>' | '~' | '/' | '.') {
            chars.next();
            let next = chars.peek().map(|&(_, c)| c);
            let (kind, two_char) = match (ch, next) {
                ('=', Some('=')) => (TokenKind::EqEq, true),
                ('=', _) => (TokenKind::Equals, false),
                ('<', Some('=')) => (TokenKind::LtEq, true),
                ('<', Some('<')) => (TokenKind::LtLt, true),
                ('<', _) => (TokenKind::Lt, false),
                ('>', Some('=')) => (TokenKind::GtEq, true),
                ('>', Some('>')) => (TokenKind::GtGt, true),
                ('>', _) => (TokenKind::Gt, false),
                ('~', Some('=')) => (TokenKind::TildeEq, true),
                ('~', _) => (TokenKind::Tilde, false),
                ('/', Some('/')) => (TokenKind::SlashSlash, true),
                ('/', _) => (TokenKind::Slash, false),
                ('.', Some('.')) => (TokenKind::DotDot, true),
                ('.', _) => return Err(LexError::Unexpected { ch, offset }),
                _ => unreachable!(),
            };
            let end = if two_char {
                chars.next();
                offset + ch.len_utf8() + 1
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
            '%' => Some(TokenKind::Percent),
            '^' => Some(TokenKind::Caret),
            '&' => Some(TokenKind::Amp),
            '|' => Some(TokenKind::Pipe),
            '#' => Some(TokenKind::Hash),
            ',' => Some(TokenKind::Comma),
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

/// Scan a numeric literal. Phase 2.2d (ADR 0023) widens this from
/// "ASCII digit run" to cover the three Lua 5.4 numeric forms our
/// subset uses: decimal integers, hex integers (`0x`/`0X` prefix),
/// decimal floats (`3.14`), and scientific notation (`1e3`,
/// `2.5e-1`). Hex floats and binary literals are deferred.
///
/// Lookahead uses byte indexing on `src` directly (the input is
/// ASCII for all numeric lexemes), advancing `chars` once per
/// committed byte so the caller's iterator stays in sync.
fn scan_number<I>(src: &str, chars: &mut std::iter::Peekable<I>) -> (Span, f64)
where
    I: Iterator<Item = (usize, char)>,
{
    let bytes = src.as_bytes();
    let start = chars.peek().expect("scan_number requires a digit").0;

    // Hex prefix: `0x` / `0X` followed by ≥1 hex digit. The prefix
    // alone (e.g. trailing `0x` with no digit) falls back to the
    // decimal scanner so the surrounding lex error is meaningful.
    if bytes.get(start) == Some(&b'0')
        && matches!(bytes.get(start + 1), Some(&b'x' | &b'X'))
        && bytes.get(start + 2).is_some_and(|b| b.is_ascii_hexdigit())
    {
        chars.next(); // '0'
        chars.next(); // 'x' / 'X'
        let mut end = start + 2;
        while bytes.get(end).is_some_and(|b| b.is_ascii_hexdigit()) {
            end += 1;
            chars.next();
        }
        let int_value = u64::from_str_radix(&src[start + 2..end], 16)
            .expect("hex digit run is always a valid u64");
        return (Span::new(start, end), int_value as f64);
    }

    // Decimal integer part — ≥1 digit (entry condition guaranteed it).
    let mut end = start;
    while bytes.get(end).is_some_and(|b| b.is_ascii_digit()) {
        end += 1;
        chars.next();
    }
    // Optional fractional part `.\d+`. Require at least one trailing
    // digit so future `t.x` field syntax doesn't grab the dot.
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(|b| b.is_ascii_digit()) {
        end += 1;
        chars.next(); // '.'
        while bytes.get(end).is_some_and(|b| b.is_ascii_digit()) {
            end += 1;
            chars.next();
        }
    }
    // Optional exponent `[eE][+-]?\d+`. Same trailing-digit guard so
    // a stray `e` doesn't get pulled in.
    if matches!(bytes.get(end), Some(&b'e' | &b'E')) {
        let mut probe = end + 1;
        if matches!(bytes.get(probe), Some(&b'+' | &b'-')) {
            probe += 1;
        }
        if bytes.get(probe).is_some_and(|b| b.is_ascii_digit()) {
            end += 1;
            chars.next(); // 'e' / 'E'
            if matches!(bytes.get(end), Some(&b'+' | &b'-')) {
                end += 1;
                chars.next();
            }
            while bytes.get(end).is_some_and(|b| b.is_ascii_digit()) {
                end += 1;
                chars.next();
            }
        }
    }
    let value = src[start..end]
        .parse::<f64>()
        .expect("scan_number recognised a valid f64 lexeme");
    (Span::new(start, end), value)
}

/// Scan a string literal `"..."` or `'...'` (Phase 2.7a, ADR 0024).
/// Recognised escapes match the Lua subset documented in the ADR:
/// `\n`, `\t`, `\r`, `\\`, `\"`, `\'`, `\0`. Anything else after a
/// backslash is `LexError::InvalidEscape`. EOF before the closing
/// quote is `LexError::UnterminatedString`.
fn scan_string<I>(
    _src: &str,
    chars: &mut std::iter::Peekable<I>,
    quote: char,
    open_offset: usize,
) -> Result<(Span, String), LexError>
where
    I: Iterator<Item = (usize, char)>,
{
    chars.next(); // consume opening quote
    let mut value = String::new();
    loop {
        let (offset, ch) = match chars.next() {
            Some(t) => t,
            None => {
                return Err(LexError::UnterminatedString {
                    offset: open_offset,
                });
            }
        };
        if ch == quote {
            // The closing quote is one byte (always ASCII for our subset).
            return Ok((Span::new(open_offset, offset + 1), value));
        }
        if ch == '\n' {
            // Lua disallows a literal newline inside a short string;
            // surface the same UnterminatedString error so the caller
            // sees the open quote location.
            return Err(LexError::UnterminatedString {
                offset: open_offset,
            });
        }
        if ch != '\\' {
            value.push(ch);
            continue;
        }
        let (esc_off, esc_ch) = match chars.next() {
            Some(t) => t,
            None => {
                return Err(LexError::UnterminatedString {
                    offset: open_offset,
                });
            }
        };
        let mapped = match esc_ch {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '0' => '\0',
            other => {
                let mut seq = String::with_capacity(2);
                seq.push('\\');
                seq.push(other);
                return Err(LexError::InvalidEscape {
                    seq,
                    offset: esc_off,
                });
            }
        };
        value.push(mapped);
    }
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
    fn lex_function_and_return_keywords_yield_keyword_tokens() {
        assert_eq!(
            kinds("function return"),
            vec![
                TokenKind::Keyword(Keyword::Function),
                TokenKind::Keyword(Keyword::Return),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_break_keyword_yields_keyword_token() {
        assert_eq!(
            kinds("break"),
            vec![TokenKind::Keyword(Keyword::Break), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_comma_yields_comma_token() {
        assert_eq!(
            kinds("1,2"),
            vec![
                TokenKind::Number(1.0),
                TokenKind::Comma,
                TokenKind::Number(2.0),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_for_keyword_yields_keyword_token() {
        assert_eq!(
            kinds("for"),
            vec![TokenKind::Keyword(Keyword::For), TokenKind::Eof],
        );
    }

    #[test]
    fn lex_logical_keywords_yield_keyword_tokens() {
        assert_eq!(
            kinds("and or not"),
            vec![
                TokenKind::Keyword(Keyword::And),
                TokenKind::Keyword(Keyword::Or),
                TokenKind::Keyword(Keyword::Not),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_control_flow_keywords_yield_keyword_tokens() {
        assert_eq!(
            kinds("if then else elseif while"),
            vec![
                TokenKind::Keyword(Keyword::If),
                TokenKind::Keyword(Keyword::Then),
                TokenKind::Keyword(Keyword::Else),
                TokenKind::Keyword(Keyword::Elseif),
                TokenKind::Keyword(Keyword::While),
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
    fn lex_tilde_alone_yields_tilde_token() {
        // Phase 2.2c (ADR 0022): standalone `~` is bitwise XOR/NOT.
        assert_eq!(kinds("~"), vec![TokenKind::Tilde, TokenKind::Eof]);
    }

    #[test]
    fn lex_bitwise_punctuation_yields_dedicated_tokens() {
        assert_eq!(
            kinds("// & | ~ << >>"),
            vec![
                TokenKind::SlashSlash,
                TokenKind::Amp,
                TokenKind::Pipe,
                TokenKind::Tilde,
                TokenKind::LtLt,
                TokenKind::GtGt,
                TokenKind::Eof,
            ]
        );
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

    // -----------------------------------------------------------
    // Phase 2.2d — number-literal extensions (ADR 0023).
    // -----------------------------------------------------------

    #[test]
    fn lex_hex_integer_literal_yields_f64() {
        assert_eq!(
            kinds("0xff"),
            vec![TokenKind::Number(255.0), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_uppercase_hex_prefix_is_accepted() {
        assert_eq!(kinds("0X1A"), vec![TokenKind::Number(26.0), TokenKind::Eof]);
    }

    #[test]
    fn lex_decimal_float_yields_f64() {
        assert_eq!(kinds("1.5"), vec![TokenKind::Number(1.5), TokenKind::Eof]);
    }

    #[test]
    fn lex_scientific_notation_with_signed_exponent() {
        assert_eq!(
            kinds("2.5e-1"),
            vec![TokenKind::Number(0.25), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_scientific_notation_without_fractional_part() {
        assert_eq!(
            kinds("1e3"),
            vec![TokenKind::Number(1000.0), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_number_does_not_pull_in_trailing_punctuation() {
        // Smoke check: a hex literal followed by `,` produces two
        // tokens (number + comma), not one. Catches a regression where
        // the scanner kept consuming non-hex bytes.
        assert_eq!(
            kinds("0xff,1"),
            vec![
                TokenKind::Number(255.0),
                TokenKind::Comma,
                TokenKind::Number(1.0),
                TokenKind::Eof,
            ]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.7a — string literals + `#` (ADR 0024).
    // -----------------------------------------------------------

    #[test]
    fn lex_double_quoted_string_yields_str_token() {
        assert_eq!(
            kinds("\"hello\""),
            vec![TokenKind::Str("hello".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_single_quoted_string_yields_str_token() {
        assert_eq!(
            kinds("'world'"),
            vec![TokenKind::Str("world".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_basic_escapes() {
        assert_eq!(
            kinds("\"a\\nb\\tc\""),
            vec![TokenKind::Str("a\nb\tc".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_escaped_backslash_and_quote() {
        assert_eq!(
            kinds("\"a\\\\b\\\"c\""),
            vec![TokenKind::Str("a\\b\"c".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_unterminated_string_returns_error() {
        let err = lex("\"oops").expect_err("missing closing quote must error");
        assert!(matches!(err, LexError::UnterminatedString { offset: 0 }));
    }

    #[test]
    fn lex_invalid_escape_returns_error() {
        let err = lex("\"a\\zb\"").expect_err("\\z is not in our escape set");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_hash_yields_dedicated_token() {
        assert_eq!(kinds("#"), vec![TokenKind::Hash, TokenKind::Eof]);
    }
}
