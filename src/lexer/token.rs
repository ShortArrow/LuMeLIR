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
    /// ADR 0197 — integer-syntax literal (`42`, `0xFF`). Phase A
    /// additive: the parser converts to `ExprKind::Number(f64)`
    /// via `as f64` cast so AST / HIR / codegen are unchanged.
    /// ADR 0198 introduces matching `ExprKind::Integer` +
    /// `ValueKind::Integer` and lifts the conversion.
    Integer(i64),
    Number(f64),
    /// String literal payload (Phase 2.7a, ADR 0024). Already
    /// escape-processed — `"a\\nb"` lexes to `Str("a\nb".into())`.
    Str(String),
    Ident(String),
    Keyword(Keyword),
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    /// `//` floor division (Phase 2.2c, ADR 0022).
    SlashSlash,
    Percent,
    Caret,
    /// `&` bitwise AND (Phase 2.2c, ADR 0022).
    Amp,
    /// `|` bitwise OR (Phase 2.2c, ADR 0022).
    Pipe,
    /// `~` standalone — bitwise XOR (binary) and bitwise NOT
    /// (unary). Disambiguated by parser context. `~=` (not-equal)
    /// is a separate token, lexed eagerly. Phase 2.2c, ADR 0022.
    Tilde,
    /// `<<` arithmetic left shift (Phase 2.2c, ADR 0022).
    LtLt,
    /// `>>` arithmetic right shift (Phase 2.2c, ADR 0022).
    GtGt,
    /// `..` string concatenation (Phase 2.7b, ADR 0025).
    DotDot,
    /// `.` field-access separator (Phase 2.6b-hash, ADR 0058).
    /// Lone `.` was a lex error until field access landed.
    Dot,
    /// `#` length operator (Phase 2.7a, ADR 0024).
    Hash,
    Equals,
    EqEq,
    TildeEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Comma,
    Semicolon,
    /// `:` method-call / method-def separator (Phase 2.6+-methods,
    /// ADR 0092). `recv:method(args)` desugars at HIR to an
    /// Index-callee Call with the receiver injected as first arg;
    /// `function recv:method(...) end` desugars to IndexAssign of
    /// a FunctionRef with implicit `self` (TaggedValue) param.
    Colon,
    /// `{` opening brace, used for table constructors
    /// (Phase 2.6a-min, ADR 0053).
    LBrace,
    /// `}` closing brace.
    RBrace,
    /// `[` opening bracket, used for array indexing
    /// (Phase 2.6a-arr, ADR 0054). Long-bracket strings still
    /// shadow this — the lexer falls through here only when
    /// `try_match_long_open` returns None.
    LBracket,
    /// `]` closing bracket (Phase 2.6a-arr, ADR 0054).
    RBracket,
    /// End-of-source sentinel. Always present as the last element of a
    /// successful [`super::lex`] result.
    Eof,
}

/// Reserved words. Grows phase by phase: 2.0 `local`, 2.1 `do`/`end`,
/// 2.2b `true`/`false`, 2.3a `nil`, 2.3b `if`/`then`/`else`/`elseif`/`while`,
/// 2.3c `and`/`or`/`not`, 2.3d `for`, 2.4 `break`, 2.5a `function`/`return`,
/// 2.4b (ADR 0035) `repeat`/`until`.
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
    Break,
    Function,
    Return,
    Repeat,
    Until,
    /// Phase 2.8e-iter-ipairs (ADR 0078): generic / sugar
    /// for-loop introducer (`for k, v in ipairs(t) do ...`).
    In,
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
            "break" => Some(Keyword::Break),
            "function" => Some(Keyword::Function),
            "return" => Some(Keyword::Return),
            "repeat" => Some(Keyword::Repeat),
            "until" => Some(Keyword::Until),
            "in" => Some(Keyword::In),
            _ => None,
        }
    }
}
