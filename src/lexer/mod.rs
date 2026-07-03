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
    let bytes = src.as_bytes();

    // Phase 2.8d (ADR 0041): a `#!` at byte 0 is a Unix shebang
    // line. Skip everything up to (but not including) the first
    // newline so the rest of the script lexes normally. Only
    // honoured at offset 0 — a `#` anywhere else stays the length
    // operator.
    if bytes.starts_with(b"#!") {
        for (_, c) in chars.by_ref() {
            if c == '\n' {
                break;
            }
        }
    }

    while let Some(&(offset, ch)) = chars.peek() {
        if ch.is_ascii_whitespace() {
            chars.next();
            continue;
        }

        // Phase 2.8a (ADR 0031) / 2.8c (ADR 0034): Lua comments.
        // Both forms are treated as whitespace post-tokenisation —
        // dispatched here, never escape into the token stream. The
        // `--[[` prefix opens a block comment; anything else after
        // `--` is a single-line comment.
        if ch == '-' && bytes.get(offset + 1) == Some(&b'-') {
            chars.next(); // first '-'
            chars.next(); // second '-'
            // Phase 2.7j (ADR 0038): `--[==[ ... ]==]` block comments
            // share the long-bracket open match with strings. Try
            // both at offset+2; on a match consume the `[==[` and
            // delegate to the level-aware scanner. Otherwise fall
            // through to the single-line skip.
            if let Some(level) = try_match_long_open(bytes, offset + 2) {
                for _ in 0..(level + 2) {
                    chars.next();
                }
                skip_block_comment(&mut chars, bytes, offset, level)?;
            } else {
                skip_line_comment(&mut chars);
            }
            continue;
        }

        if ch.is_ascii_digit() {
            let (span, lit) = scan_number(src, &mut chars);
            let kind = match lit {
                NumericLit::Integer(i) => TokenKind::Integer(i),
                NumericLit::Float(f) => TokenKind::Number(f),
            };
            tokens.push(Token::new(kind, span));
            continue;
        }

        if matches!(ch, '"' | '\'') {
            let (span, s) = scan_string(src, &mut chars, ch, offset)?;
            tokens.push(Token::new(TokenKind::Str(s), span));
            continue;
        }

        // Phase 2.7j (ADR 0038): `[==[ ... ]==]` long-bracket
        // string takes priority over `[` indexing. Phase 2.6a-arr
        // (ADR 0054): when the long-bracket match fails, fall
        // through to emit a `LBracket` token for array indexing.
        if ch == '[' {
            if let Some(level) = try_match_long_open(bytes, offset) {
                for _ in 0..(level + 2) {
                    chars.next();
                }
                let body = scan_long_bracket_body(&mut chars, bytes, level)
                    .map_err(|_| LexError::UnterminatedBracket { offset })?;
                // The body scanner consumed up to and including the
                // closing `]==]`. The next char's offset (or EOF)
                // is therefore the token's end.
                let end = chars.peek().map(|&(o, _)| o).unwrap_or(src.len());
                tokens.push(Token::new(TokenKind::Str(body), Span::new(offset, end)));
                continue;
            }
            // Bare `[` — fall through to single-token dispatch.
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
                // Phase 2.6b-hash (ADR 0058): lone `.` is now a real
                // token for field access. `.NUMBER` (`.5` etc.) is
                // already protected — `scan_number` only consumes
                // a fractional `.` when it has trailing digits, so
                // the lexer reaches this dispatch only for genuine
                // field-access-or-error contexts.
                ('.', _) => (TokenKind::Dot, false),
                _ => unreachable!(),
            };
            // ADR 0293 — F1-A: `...` varargs token. `..` above may
            // extend to `...` if a third dot follows.
            if matches!(kind, TokenKind::DotDot) {
                chars.next(); // consume the second `.`
                if matches!(chars.peek(), Some((_, '.'))) {
                    chars.next(); // consume the third `.`
                    let end = offset + 3;
                    tokens.push(Token::new(TokenKind::DotDotDot, Span::new(offset, end)));
                    continue;
                }
                // Not `...` — emit `..` with the already-consumed second dot.
                let end = offset + 2;
                tokens.push(Token::new(TokenKind::DotDot, Span::new(offset, end)));
                continue;
            }
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
            // Phase 2.6+-methods (ADR 0092): method-call / method-def
            // separator. Lua's `::label::` is not supported; double-colon
            // lexes as two `Colon` tokens and the parser rejects.
            ':' => Some(TokenKind::Colon),
            // Phase 2.6a-min (ADR 0053): table constructor delimiters.
            '{' => Some(TokenKind::LBrace),
            '}' => Some(TokenKind::RBrace),
            // Phase 2.6a-arr (ADR 0054): array index delimiters.
            // `[` reaches here only when the preceding long-bracket
            // string scan failed.
            '[' => Some(TokenKind::LBracket),
            ']' => Some(TokenKind::RBracket),
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

/// Phase 2.8a (ADR 0031): consume a `-- ...` single-line comment
/// up to (but not including) the next newline or EOF. The two
/// leading `-`s are consumed by the caller so this helper sees the
/// comment's payload only. Pure relative to its inputs — the only
/// effect is advancing `chars`.
fn skip_line_comment<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = (usize, char)>,
{
    while let Some(&(_, c)) = chars.peek() {
        if c == '\n' {
            break;
        }
        chars.next();
    }
}

/// Phase 2.7j (ADR 0038): pattern-match a long-bracket opener
/// starting at byte position `at`. Returns `Some(level)` when
/// `bytes[at..]` begins with `[` + `=`*level + `[`, else `None`.
/// Pure byte-index arithmetic; nothing is consumed.
fn try_match_long_open(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut level = 0usize;
    while bytes.get(at + 1 + level) == Some(&b'=') {
        level += 1;
    }
    if bytes.get(at + 1 + level) == Some(&b'[') {
        Some(level)
    } else {
        None
    }
}

/// Phase 2.8c / 2.7j (ADRs 0034, 0038): generic long-bracket body
/// scanner. Reads characters from `chars` until it finds a matching
/// `]` + `=`*level + `]` close, returning the accumulated body
/// (`Ok(String)`) or `Err(())` on EOF before the close.
///
/// The opening `[` + `=`*level + `[` is consumed by the caller so
/// this helper sees the body only. Lookahead for the close uses
/// `bytes` (the source as bytes) — long brackets contain raw text,
/// so we just count `=`s after a `]` to test for a level match.
/// Per Lua spec, an immediate leading `\n` after the opener is
/// stripped from the body.
///
/// Returning `Err(())` lets callers map to the variant they prefer
/// (`UnterminatedBracket` for strings, `UnterminatedComment` for
/// `--[[...]]` block comments). Pure relative to its inputs save
/// for `chars` advancement.
fn scan_long_bracket_body<I>(
    chars: &mut std::iter::Peekable<I>,
    bytes: &[u8],
    level: usize,
) -> Result<String, ()>
where
    I: Iterator<Item = (usize, char)>,
{
    // Lua spec: leading `\n` immediately after the opener is dropped.
    if matches!(chars.peek(), Some(&(_, '\n'))) {
        chars.next();
    }
    let mut body = String::new();
    loop {
        let (offset, c) = chars.next().ok_or(())?;
        if c == ']' {
            let mut k = 0usize;
            while bytes.get(offset + 1 + k) == Some(&b'=') {
                k += 1;
            }
            if k == level && bytes.get(offset + 1 + k) == Some(&b']') {
                // Skip the matched `=`s and the final `]`.
                for _ in 0..(level + 1) {
                    chars.next();
                }
                return Ok(body);
            }
        }
        body.push(c);
    }
}

/// Phase 2.8c (ADR 0034) / 2.7j (ADR 0038): consume a `--[==[ ... ]==]`
/// block comment of the given bracket level. `open_offset` is the
/// byte offset of the leading `-` so the diagnostic points back at
/// the comment's start. Delegates to `scan_long_bracket_body` and
/// discards the result.
fn skip_block_comment<I>(
    chars: &mut std::iter::Peekable<I>,
    bytes: &[u8],
    open_offset: usize,
    level: usize,
) -> Result<(), LexError>
where
    I: Iterator<Item = (usize, char)>,
{
    scan_long_bracket_body(chars, bytes, level).map_err(|_| LexError::UnterminatedComment {
        offset: open_offset,
    })?;
    Ok(())
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

/// ADR 0197 — output of [`scan_number`]. Carries the lexer's
/// integer-vs-float syntactic decision before the parser demotes
/// (Phase A) or routes to a dedicated AST variant (ADR 0198+).
pub(crate) enum NumericLit {
    Integer(i64),
    Float(f64),
}

/// Scan a numeric literal. Phase 2.2d (ADR 0023) widens this from
/// "ASCII digit run" to cover the three Lua 5.4 numeric forms our
/// subset uses: decimal integers, hex integers (`0x`/`0X` prefix),
/// decimal floats (`3.14`), and scientific notation (`1e3`,
/// `2.5e-1`). Hex floats and binary literals are deferred.
///
/// ADR 0197 — returns `NumericLit::Integer(i64)` for integer-syntax
/// lexemes (decimal digits without `.`/`eE`, or hex `0x...`) and
/// `NumericLit::Float(f64)` for fractional / scientific notation.
/// Decimal integer overflow falls back to `Float` per Lua §3.4.3.
///
/// Lookahead uses byte indexing on `src` directly (the input is
/// ASCII for all numeric lexemes), advancing `chars` once per
/// committed byte so the caller's iterator stays in sync.
fn scan_number<I>(src: &str, chars: &mut std::iter::Peekable<I>) -> (Span, NumericLit)
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
        // ADR 0197 — Lua 5.4 §3.1: hex integer literals are signed
        // 64-bit. `u64 → i64 as` preserves bits so 0xFFFFFFFFFFFFFFFF
        // becomes -1, matching the spec.
        let int_value = u64::from_str_radix(&src[start + 2..end], 16)
            .expect("hex digit run is always a valid u64");
        return (Span::new(start, end), NumericLit::Integer(int_value as i64));
    }

    // Decimal integer part — ≥1 digit (entry condition guaranteed it).
    let mut end = start;
    while bytes.get(end).is_some_and(|b| b.is_ascii_digit()) {
        end += 1;
        chars.next();
    }
    let int_part_end = end;
    let mut is_float = false;
    // Optional fractional part `.\d+`. Require at least one trailing
    // digit so future `t.x` field syntax doesn't grab the dot.
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(|b| b.is_ascii_digit()) {
        is_float = true;
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
            is_float = true;
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
    // ADR 0197 — pure decimal integer (no `.` no `eE`): try i64; if
    // it overflows, fall back to f64 per Lua §3.4.3 (source-literal
    // integer overflow → float).
    if !is_float {
        if let Ok(i) = src[start..int_part_end].parse::<i64>() {
            return (Span::new(start, end), NumericLit::Integer(i));
        }
    }
    let value = src[start..end]
        .parse::<f64>()
        .expect("scan_number recognised a valid f64 lexeme");
    (Span::new(start, end), NumericLit::Float(value))
}

/// Scan a string literal `"..."` or `'...'` (Phase 2.7a, ADR 0024).
/// Recognised escapes match the Lua subset documented in ADRs 0024,
/// 0039, and 0040: `\n`, `\t`, `\r`, `\\`, `\"`, `\'`, `\a`, `\b`,
/// `\f`, `\v`, `\xHH` (hex byte, 0..=0x7F), `\ddd` (1-3 decimal
/// digits, 0..=127), `\u{XXXX}` (Unicode codepoint → UTF-8), and
/// `\z` (skip a run of whitespace, including newlines). Anything
/// else after a backslash is `LexError::InvalidEscape`. EOF before
/// the closing quote is `LexError::UnterminatedString`.
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
        // Phase 2.7l (ADR 0040): `\z` skips a run of whitespace —
        // including newlines — letting source split a long string
        // across lines without embedding the line break. Handled
        // before the match so it can short-circuit `value.push`.
        if esc_ch == 'z' {
            while let Some(&(_, c)) = chars.peek() {
                if c.is_ascii_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            continue;
        }
        let mapped = match esc_ch {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            // Phase 2.7k (ADR 0039): C-style single-char escapes.
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0C',
            'v' => '\x0B',
            // Phase 2.7k (ADR 0039): `\xHH` hex byte.
            'x' => read_hex_escape(chars, esc_off)?,
            // Phase 2.7l (ADR 0040): `\u{XXXX}` Unicode codepoint
            // → UTF-8 bytes. Rust's `char` *is* a codepoint, so
            // `String::push(char)` does the UTF-8 encoding for us.
            'u' => read_unicode_escape(chars, esc_off)?,
            // Phase 2.7k (ADR 0039): `\ddd` decimal byte. The first
            // digit was already consumed; the helper reads up to two
            // more.
            d if d.is_ascii_digit() => read_decimal_escape(chars, d, esc_off)?,
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

/// Phase 2.7k (ADR 0039): read exactly two hex digits after `\x` and
/// return the resulting byte as a `char`. Restricts to the
/// ASCII-safe range 0x00..=0x7F so values flow cleanly through Rust
/// `String` (UTF-8) without splitting into multi-byte sequences.
/// `esc_off` points at the `x` so diagnostics carry that location.
fn read_hex_escape<I>(chars: &mut std::iter::Peekable<I>, esc_off: usize) -> Result<char, LexError>
where
    I: Iterator<Item = (usize, char)>,
{
    let d1 = chars.next().and_then(|(_, c)| c.to_digit(16));
    let d2 = chars.next().and_then(|(_, c)| c.to_digit(16));
    match (d1, d2) {
        (Some(a), Some(b)) => {
            let v = a * 16 + b;
            if v > 0x7F {
                Err(LexError::InvalidEscape {
                    seq: format!("\\x{v:02x}"),
                    offset: esc_off,
                })
            } else {
                Ok(v as u8 as char)
            }
        }
        _ => Err(LexError::InvalidEscape {
            seq: "\\x".into(),
            offset: esc_off,
        }),
    }
}

/// Phase 2.7k (ADR 0039): read a 1-3 digit decimal escape. The
/// caller has already consumed the first digit and passes it as
/// `first`. Subsequent digits are taken greedily; scanning stops at
/// the first non-digit (which remains in the iterator). Restricts
/// to 0..=127 so the value fits in one UTF-8 byte.
fn read_decimal_escape<I>(
    chars: &mut std::iter::Peekable<I>,
    first: char,
    esc_off: usize,
) -> Result<char, LexError>
where
    I: Iterator<Item = (usize, char)>,
{
    let mut value: u32 = first.to_digit(10).expect("first must be a digit");
    for _ in 0..2 {
        match chars.peek() {
            Some(&(_, c)) if c.is_ascii_digit() => {
                value = value * 10 + c.to_digit(10).unwrap();
                chars.next();
            }
            _ => break,
        }
    }
    if value > 127 {
        Err(LexError::InvalidEscape {
            seq: format!("\\{value}"),
            offset: esc_off,
        })
    } else {
        Ok(value as u8 as char)
    }
}

/// Phase 2.7l (ADR 0040): read `\u{XXXX}` and return the codepoint
/// as a Rust `char`. Caller pushes via `String::push`, which
/// UTF-8-encodes automatically. Accepts 1+ hex digits between the
/// braces; rejects empty digit runs, missing braces, codepoints
/// outside the Unicode scalar range (surrogates 0xD800..=0xDFFF
/// and values >0x10FFFF). `esc_off` points at the `u` for
/// diagnostics.
fn read_unicode_escape<I>(
    chars: &mut std::iter::Peekable<I>,
    esc_off: usize,
) -> Result<char, LexError>
where
    I: Iterator<Item = (usize, char)>,
{
    if !matches!(chars.next(), Some((_, '{'))) {
        return Err(LexError::InvalidEscape {
            seq: "\\u".into(),
            offset: esc_off,
        });
    }
    let mut value: u32 = 0;
    let mut digits = 0usize;
    loop {
        match chars.peek() {
            Some(&(_, '}')) => {
                chars.next();
                break;
            }
            Some(&(_, c)) if c.is_ascii_hexdigit() => {
                value = value
                    .checked_mul(16)
                    .and_then(|v| v.checked_add(c.to_digit(16).unwrap()))
                    .ok_or_else(|| LexError::InvalidEscape {
                        seq: "\\u{...}".into(),
                        offset: esc_off,
                    })?;
                digits += 1;
                chars.next();
            }
            _ => {
                return Err(LexError::InvalidEscape {
                    seq: "\\u{".into(),
                    offset: esc_off,
                });
            }
        }
    }
    if digits == 0 {
        return Err(LexError::InvalidEscape {
            seq: "\\u{}".into(),
            offset: esc_off,
        });
    }
    char::from_u32(value).ok_or_else(|| LexError::InvalidEscape {
        seq: format!("\\u{{{value:x}}}"),
        offset: esc_off,
    })
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
        assert_eq!(kinds("42"), vec![TokenKind::Integer(42), TokenKind::Eof]);
    }

    #[test]
    fn lex_plus_between_integers_yields_three_tokens() {
        assert_eq!(
            kinds("1+2"),
            vec![
                TokenKind::Integer(1),
                TokenKind::Plus,
                TokenKind::Integer(2),
                TokenKind::Eof,
            ],
        );
    }

    #[test]
    fn lex_whitespace_is_skipped() {
        assert_eq!(
            kinds(" 1 +\t2\n"),
            vec![
                TokenKind::Integer(1),
                TokenKind::Plus,
                TokenKind::Integer(2),
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
                TokenKind::Integer(1),
                TokenKind::Plus,
                TokenKind::Integer(2),
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
                TokenKind::Integer(1),
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
                TokenKind::Integer(1),
                TokenKind::Comma,
                TokenKind::Integer(2),
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
        assert_eq!(kinds("0xff"), vec![TokenKind::Integer(255), TokenKind::Eof]);
    }

    #[test]
    fn lex_uppercase_hex_prefix_is_accepted() {
        assert_eq!(kinds("0X1A"), vec![TokenKind::Integer(26), TokenKind::Eof]);
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
        // ADR 0197 — scientific notation (`1e3`) is float syntax
        // even when the value is integer-valued. Stays as
        // `TokenKind::Number(f64)`.
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
                TokenKind::Integer(255),
                TokenKind::Comma,
                TokenKind::Integer(1),
                TokenKind::Eof,
            ]
        );
    }

    // ADR 0197 — integer-syntax token distinction.

    #[test]
    fn lex_integer_decimal_emits_integer_token() {
        assert_eq!(kinds("42"), vec![TokenKind::Integer(42), TokenKind::Eof]);
    }

    #[test]
    fn lex_integer_hex_emits_integer_token() {
        assert_eq!(kinds("0xFF"), vec![TokenKind::Integer(255), TokenKind::Eof]);
    }

    #[test]
    fn lex_float_decimal_emits_number_token() {
        assert_eq!(kinds("42.5"), vec![TokenKind::Number(42.5), TokenKind::Eof]);
    }

    #[test]
    fn lex_float_scientific_emits_number_token() {
        assert_eq!(
            kinds("2.5e2"),
            vec![TokenKind::Number(250.0), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_integer_overflow_falls_back_to_float() {
        // ADR 0197 — i64 overflow on decimal integer literal falls
        // back to float per Lua §3.4.3.
        let toks = kinds("99999999999999999999");
        assert!(
            matches!(toks[0], TokenKind::Number(_)),
            "expected Number fallback, got {:?}",
            toks[0]
        );
    }

    #[test]
    fn lex_hex_max_u64_wraps_to_negative_i64() {
        // ADR 0197 — `u64 → i64 as` preserves bits per Lua spec.
        assert_eq!(
            kinds("0xFFFFFFFFFFFFFFFF"),
            vec![TokenKind::Integer(-1), TokenKind::Eof]
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
        // `\q` is unrecognised. (`\z` was the original sample but
        // became valid in Phase 2.7l (ADR 0040).)
        let err = lex("\"a\\qb\"").expect_err("\\q is not in our escape set");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_hash_yields_dedicated_token() {
        assert_eq!(kinds("#"), vec![TokenKind::Hash, TokenKind::Eof]);
    }

    // -----------------------------------------------------------
    // Phase 2.8a — single-line comments (ADR 0031).
    // -----------------------------------------------------------

    #[test]
    fn lex_single_line_comment_to_eof_is_skipped() {
        assert_eq!(kinds("-- comment"), vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_single_line_comment_terminates_at_newline() {
        assert_eq!(
            kinds("-- skip\n42"),
            vec![TokenKind::Integer(42), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_inline_comment_after_expression() {
        assert_eq!(
            kinds("1 + 2 -- sum\n3"),
            vec![
                TokenKind::Integer(1),
                TokenKind::Plus,
                TokenKind::Integer(2),
                TokenKind::Integer(3),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_single_minus_is_not_a_comment() {
        // A bare `-` keeps its arithmetic-minus meaning.
        assert_eq!(
            kinds("- 1"),
            vec![TokenKind::Minus, TokenKind::Integer(1), TokenKind::Eof]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.8c — block comments `--[[ ... ]]` (ADR 0034).
    // -----------------------------------------------------------

    #[test]
    fn lex_block_comment_inline_is_skipped() {
        assert_eq!(
            kinds("1 --[[ in line ]] 2"),
            vec![TokenKind::Integer(1), TokenKind::Integer(2), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_block_comment_spanning_multiple_lines_is_skipped() {
        let src = "1\n--[[\nline two\nline three\n]]\n2";
        assert_eq!(
            kinds(src),
            vec![TokenKind::Integer(1), TokenKind::Integer(2), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_block_comment_only_file_yields_empty_token_stream() {
        assert_eq!(kinds("--[[ all comment ]]"), vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_unterminated_block_comment_returns_error() {
        let err = lex("--[[ no close").expect_err("missing `]]` must error");
        assert!(matches!(err, LexError::UnterminatedComment { .. }));
    }

    #[test]
    fn lex_dash_dash_open_bracket_only_one_is_line_comment() {
        // `--[ x` (single bracket) is a regular line comment that
        // happens to start with `[`. It is NOT a block comment.
        assert_eq!(
            kinds("1 --[ rest of line\n2"),
            vec![TokenKind::Integer(1), TokenKind::Integer(2), TokenKind::Eof]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.4b — `repeat` / `until` keywords (ADR 0035).
    // -----------------------------------------------------------

    #[test]
    fn lex_repeat_keyword() {
        assert_eq!(
            kinds("repeat"),
            vec![TokenKind::Keyword(Keyword::Repeat), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_until_keyword() {
        assert_eq!(
            kinds("until"),
            vec![TokenKind::Keyword(Keyword::Until), TokenKind::Eof]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.7j — long-bracket strings + level-N comments (ADR 0038).
    // -----------------------------------------------------------

    #[test]
    fn lex_level0_long_bracket_string() {
        assert_eq!(
            kinds("[[hello]]"),
            vec![TokenKind::Str("hello".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_level1_long_bracket_string() {
        // Level-1 brackets `[=[ ... ]=]` allow `]]` inside the body.
        assert_eq!(
            kinds("[=[has ]] inside]=]"),
            vec![TokenKind::Str("has ]] inside".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_long_bracket_strips_leading_newline() {
        // Lua spec: an immediate `\n` after the opener is dropped.
        assert_eq!(
            kinds("[[\nhello]]"),
            vec![TokenKind::Str("hello".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_long_bracket_keeps_internal_newlines() {
        assert_eq!(
            kinds("[[a\nb]]"),
            vec![TokenKind::Str("a\nb".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_long_bracket_does_not_process_escapes() {
        // Body is raw — `\n` is two chars, not LF.
        assert_eq!(
            kinds("[[a\\nb]]"),
            vec![TokenKind::Str("a\\nb".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_unterminated_long_bracket_returns_error() {
        let err = lex("[[no close").expect_err("missing `]]` must error");
        assert!(matches!(err, LexError::UnterminatedBracket { .. }));
    }

    #[test]
    fn lex_level1_block_comment_is_skipped() {
        // Level-1 block comment lets `]]` survive in the body.
        assert_eq!(
            kinds("--[=[ has ]] inside ]=] 1"),
            vec![TokenKind::Integer(1), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_open_bracket_without_long_form_emits_lbracket_after_2_6a_arr() {
        // Phase 2.6a-arr (ADR 0054): `[` is now a real token —
        // long-bracket scan still has priority, but a bare `[`
        // emits LBracket for array indexing.
        assert_eq!(
            kinds("[ x]"),
            vec![
                TokenKind::LBracket,
                TokenKind::Ident("x".into()),
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.7k — extended string escapes (ADR 0039).
    // `\a \b \f \v` (single-char), `\xHH` (hex byte, 0..=0x7F),
    // `\ddd` (decimal byte, 0..=127, 1-3 digits).
    // -----------------------------------------------------------

    #[test]
    fn lex_string_with_alert_and_backspace_escapes() {
        assert_eq!(
            kinds("\"\\a\\b\""),
            vec![TokenKind::Str("\x07\x08".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_form_feed_and_vtab_escapes() {
        assert_eq!(
            kinds("\"\\f\\v\""),
            vec![TokenKind::Str("\x0C\x0B".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_hex_escape() {
        // `\x41` = 'A'.
        assert_eq!(
            kinds("\"\\x41\""),
            vec![TokenKind::Str("A".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_hex_escape_lowercase_digits() {
        // Hex digits accept either case.
        assert_eq!(
            kinds("\"\\x7e\""),
            vec![TokenKind::Str("~".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_decimal_escape_three_digits() {
        // `\065` = 'A'.
        assert_eq!(
            kinds("\"\\065\""),
            vec![TokenKind::Str("A".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_decimal_escape_short_terminates_at_non_digit() {
        // `\65` followed by 'X' — the decimal scanner must stop at
        // the non-digit, leaving 'X' as part of the string.
        assert_eq!(
            kinds("\"\\65X\""),
            vec![TokenKind::Str("AX".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_hex_escape_out_of_ascii_range_is_error() {
        // `\xff` = 255 > 127, deferred until raw-byte string support.
        let err = lex("\"\\xff\"").expect_err("0x80+ hex escapes deferred");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_string_with_decimal_escape_out_of_ascii_range_is_error() {
        // `\200` = 200 > 127, same restriction.
        let err = lex("\"\\200\"").expect_err("decimal escapes >127 deferred");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_string_with_short_hex_escape_is_error() {
        // `\xZ` — first hex digit invalid.
        let err = lex("\"\\xZ\"").expect_err("\\x must be followed by 2 hex digits");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    // -----------------------------------------------------------
    // Phase 2.7l — `\u{XXXX}` codepoint and `\z` whitespace skip
    // (ADR 0040).
    // -----------------------------------------------------------

    #[test]
    fn lex_string_with_unicode_escape_ascii() {
        // `\u{41}` = 'A' (1 UTF-8 byte).
        assert_eq!(
            kinds("\"\\u{41}\""),
            vec![TokenKind::Str("A".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_unicode_escape_two_byte() {
        // `\u{e9}` = 'é' (2 UTF-8 bytes 0xC3 0xA9).
        assert_eq!(
            kinds("\"\\u{e9}\""),
            vec![TokenKind::Str("é".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_unicode_escape_three_byte() {
        // `\u{3042}` = 'あ' (3 UTF-8 bytes).
        assert_eq!(
            kinds("\"\\u{3042}\""),
            vec![TokenKind::Str("あ".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_unicode_escape_emoji() {
        // `\u{1F600}` = 😀 (4 UTF-8 bytes).
        assert_eq!(
            kinds("\"\\u{1F600}\""),
            vec![TokenKind::Str("😀".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_unicode_escape_missing_open_brace_is_error() {
        let err = lex("\"\\u41\"").expect_err("\\u requires `{`");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_string_with_unicode_escape_missing_close_brace_is_error() {
        let err = lex("\"\\u{41\"").expect_err("\\u{ requires closing `}`");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_string_with_unicode_escape_invalid_codepoint_is_error() {
        // 0xD800 is a UTF-16 surrogate, invalid as a Unicode scalar.
        let err = lex("\"\\u{D800}\"").expect_err("surrogates are not valid scalars");
        assert!(matches!(err, LexError::InvalidEscape { .. }));
    }

    #[test]
    fn lex_string_with_z_skip_consumes_whitespace_run() {
        // `\z` skips the `\n` + spaces run; the result is "ab".
        assert_eq!(
            kinds("\"a\\z   \n  b\""),
            vec![TokenKind::Str("ab".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lex_string_with_z_at_eos_is_noop() {
        // `\z` followed immediately by the closing quote: nothing
        // to skip, the string is just "a".
        assert_eq!(
            kinds("\"a\\z\""),
            vec![TokenKind::Str("a".into()), TokenKind::Eof]
        );
    }

    // -----------------------------------------------------------
    // Phase 2.8d — `#!` shebang line skip (ADR 0041).
    // -----------------------------------------------------------

    #[test]
    fn lex_shebang_at_offset_zero_is_skipped() {
        // The `#!/usr/bin/env lumelir` header is dropped; the
        // following `print(1)` lexes normally.
        assert_eq!(
            kinds("#!/usr/bin/env lumelir\nprint(1)"),
            vec![
                TokenKind::Ident("print".into()),
                TokenKind::LParen,
                TokenKind::Integer(1),
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_shebang_only_yields_eof() {
        assert_eq!(kinds("#!/usr/bin/env lua"), vec![TokenKind::Eof]);
    }

    #[test]
    fn lex_hash_not_at_offset_zero_remains_length_operator() {
        // A `#` that is not at byte 0 is still the length operator,
        // even when followed by `!` (which surfaces as Unexpected).
        let err = lex("local x = 1\n#!").expect_err("non-leading `#!` is not shebang");
        assert!(matches!(err, LexError::Unexpected { ch: '!', .. }));
    }

    #[test]
    fn lex_leading_hash_without_bang_is_length_operator() {
        // `#x` at offset 0 is `Hash` + `Ident("x")`, not shebang.
        assert_eq!(
            kinds("#x"),
            vec![
                TokenKind::Hash,
                TokenKind::Ident("x".into()),
                TokenKind::Eof
            ]
        );
    }
}
