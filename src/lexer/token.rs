/// Byte offsets into the original source string.
///
/// `end` is exclusive, matching Rust's slice semantics: `src[span.start..span.end]`
/// yields the token's lexeme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// A token produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub const fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The discriminated category of a [`Token`].
///
/// Grows as more of the Lua 5.4 lexical grammar is covered. See ADRs 0001
/// (Phase 1 baseline) and 0007 (Phase 2.0 additions).
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Number(f64),
    Ident(String),
    Keyword(Keyword),
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Equals,
    EqEq,
    TildeEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Comma,
    Semicolon,
    /// End-of-source sentinel. Always present as the last element of a
    /// successful [`super::lex`] result.
    Eof,
}

/// Reserved words. Grows phase by phase: 2.0 `local`, 2.1 `do`/`end`,
/// 2.2b `true`/`false`, 2.3a `nil`, 2.3b `if`/`then`/`else`/`elseif`/`while`,
/// 2.3c `and`/`or`/`not`, 2.3d `for`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Local,
    Do,
    End,
    True,
    False,
    Nil,
    If,
    Then,
    Else,
    Elseif,
    While,
    And,
    Or,
    Not,
    For,
}

impl Keyword {
    pub fn from_lexeme(lex: &str) -> Option<Self> {
        match lex {
            "local" => Some(Keyword::Local),
            "do" => Some(Keyword::Do),
            "end" => Some(Keyword::End),
            "true" => Some(Keyword::True),
            "false" => Some(Keyword::False),
            "nil" => Some(Keyword::Nil),
            "if" => Some(Keyword::If),
            "then" => Some(Keyword::Then),
            "else" => Some(Keyword::Else),
            "elseif" => Some(Keyword::Elseif),
            "while" => Some(Keyword::While),
            "and" => Some(Keyword::And),
            "or" => Some(Keyword::Or),
            "not" => Some(Keyword::Not),
            "for" => Some(Keyword::For),
            _ => None,
        }
    }
}
