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
/// Kept intentionally small for Phase 1 PoC (`print(1 + 2)`). Extended in
/// subsequent phases as more of the Lua 5.4 lexical grammar is covered.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Numeric literal stored as `f64` for now. Integer / float distinction
    /// will be re-introduced when Lua 5.4 number semantics are covered.
    Number(f64),
    /// Identifier (including keyword-like names; keyword classification is
    /// deferred to a later phase).
    Ident(String),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `+`
    Plus,
    /// End-of-source sentinel. Always present as the last element of a
    /// successful [`super::lex`] result.
    Eof,
}
