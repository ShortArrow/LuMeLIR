//! Phase 2.9a (ADR 0045): user-facing diagnostic rendering.
//!
//! Lower layers (lexer / parser / HIR) carry **byte offsets** in
//! their error variants, which is the right level for those layers
//! — the offset is exact and stable across encodings. The CLI
//! boundary translates each offset into a human `(line, column)`
//! pair and prepends the source path. Pure helpers here, no I/O.

use std::path::Path;

/// 1-based-line, 1-based-column for a byte offset into `src`.
/// Lines are bounded by `\n`; columns count Unicode scalar values
/// from the start of the line, matching how rustc renders.
///
/// Out-of-range offsets clamp to `src.len()` so error rendering
/// never panics.
pub fn offset_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(src.len());
    let prefix = &src[..clamped];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = src[line_start..clamped].chars().count() + 1;
    (line, col)
}

/// Errors that carry a byte offset into the source they were
/// produced from. Implemented for `LexError`, `ParseError`, and
/// `HirError`; the CLI uses it to render each error with location.
pub trait HasOffset {
    fn offset(&self) -> usize;
}

impl HasOffset for crate::lexer::LexError {
    fn offset(&self) -> usize {
        self.offset()
    }
}

impl HasOffset for crate::parser::ParseError {
    fn offset(&self) -> usize {
        self.offset()
    }
}

impl HasOffset for crate::hir::HirError {
    fn offset(&self) -> usize {
        self.offset()
    }
}

/// Render an error as `path:line:col: <layer> error: <display>`,
/// followed by a rustc-style source snippet pointing a caret at
/// the offending column. `layer` is a short tag like "parse" or
/// "hir" so users can tell at a glance which stage rejected
/// their source.
pub fn format_error<E: HasOffset + std::fmt::Display>(
    src: &str,
    path: &Path,
    layer: &str,
    err: &E,
) -> String {
    let offset = err.offset();
    let (line, col) = offset_to_line_col(src, offset);
    let snip = snippet(src, offset);
    format!(
        "{}:{}:{}: {} error: {}\n{}",
        path.display(),
        line,
        col,
        layer,
        err,
        snip
    )
}

/// Phase 2.9b (ADR 0046): render the source line containing
/// `offset` and a caret aligned under the corresponding column.
/// Output is exactly two lines plus a trailing newline:
///
/// ```text
///   | <source line>
///   |   <caret>
/// ```
///
/// Caret column matches `offset_to_line_col`'s column count
/// (Unicode scalar values from line start). Out-of-range offsets
/// clamp to EOF, placing the caret one past the last char of the
/// last line. Empty source produces a blank source line and a
/// caret at column 1.
pub fn snippet(src: &str, offset: usize) -> String {
    let clamped = offset.min(src.len());
    let line_start = src[..clamped].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = src[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(src.len());
    let line = &src[line_start..line_end];
    let col_chars = src[line_start..clamped].chars().count();
    let mut out = String::new();
    out.push_str("  | ");
    out.push_str(line);
    out.push('\n');
    out.push_str("  | ");
    for _ in 0..col_chars {
        out.push(' ');
    }
    out.push('^');
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_single_line_source() {
        // 1-col-wide caret at the offset; gutter keeps two spaces.
        let s = snippet("abc", 1);
        assert_eq!(s, "  | abc\n  |  ^\n");
    }

    #[test]
    fn snippet_caret_at_first_column() {
        let s = snippet("xyz", 0);
        assert_eq!(s, "  | xyz\n  | ^\n");
    }

    #[test]
    fn snippet_picks_offending_line_in_multi_line_source() {
        // The error is on line 2 ("def"), col 2.
        let s = snippet("abc\ndef\nghi", 5);
        assert_eq!(s, "  | def\n  |  ^\n");
    }

    #[test]
    fn snippet_caret_aligns_under_multibyte_char() {
        // 'あ' is 1 char wide. Offset 4 lands just after it in
        // "aあb" (byte 4 == 'b'). Caret should be at col 3.
        let s = snippet("aあb", 4);
        assert_eq!(s, "  | aあb\n  |   ^\n");
    }

    #[test]
    fn snippet_clamps_offset_past_eof_to_last_line() {
        // Past EOF — caret lands one past the last char of the
        // last line.
        let s = snippet("abc", 99);
        assert_eq!(s, "  | abc\n  |    ^\n");
    }

    #[test]
    fn snippet_empty_source_emits_blank_line_and_caret() {
        // Edge case — a totally empty source still renders a
        // valid two-line snippet so the caller's format string
        // doesn't have to special-case it.
        let s = snippet("", 0);
        assert_eq!(s, "  | \n  | ^\n");
    }

    #[test]
    fn offset_zero_is_line_one_col_one() {
        assert_eq!(offset_to_line_col("abc", 0), (1, 1));
    }

    #[test]
    fn offset_after_first_newline_starts_line_two() {
        assert_eq!(offset_to_line_col("abc\nxyz", 4), (2, 1));
    }

    #[test]
    fn offset_at_third_line_first_char() {
        assert_eq!(offset_to_line_col("a\nb\nc", 4), (3, 1));
    }

    #[test]
    fn col_advances_within_a_line() {
        // "abc": offset 2 → col 3 (1-based).
        assert_eq!(offset_to_line_col("abc", 2), (1, 3));
    }

    #[test]
    fn col_counts_chars_not_bytes() {
        // 'あ' is 3 UTF-8 bytes. Offset 4 lands just after it,
        // which is line 1, col 3 (1-based: 'a' at col 1, 'あ'
        // at col 2, position-after-あ at col 3).
        assert_eq!(offset_to_line_col("aあb", 4), (1, 3));
    }

    #[test]
    fn out_of_range_offset_clamps_to_eof() {
        // No panic. Reports the position right after the last char.
        let (line, col) = offset_to_line_col("abc", 999);
        assert_eq!(line, 1);
        assert_eq!(col, 4);
    }

    #[test]
    fn empty_source_offset_zero_is_line_one_col_one() {
        assert_eq!(offset_to_line_col("", 0), (1, 1));
    }

    #[test]
    fn newline_at_end_doesnt_double_count_line() {
        // Offset right at the trailing newline byte is still on
        // line 1 (the newline closes line 1, doesn't open line 2
        // until the *next* byte).
        assert_eq!(offset_to_line_col("abc\n", 3), (1, 4));
    }
}
